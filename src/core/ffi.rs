//! Ownership-safe C entry points for the portable core and its views.
//!
//! The ABI deliberately exposes integer tokens rather than Rust pointers.
//! Tokens are process-local, nonzero, and never reused. Registry locks are
//! held only long enough to check a core out for one serial
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

mod formatting;
pub use formatting::*;
mod argument_list;
pub use argument_list::*;
mod external_change;
pub use external_change::*;
mod completion;
pub use completion::*;
mod substitute_confirmation;
pub use substitute_confirmation::*;
mod search;
pub use search::*;
mod prelayout;
pub use prelayout::*;
mod whitespace;
pub use whitespace::*;
mod html_export;
pub use html_export::*;
mod startup;
mod ex_files;
pub use ex_files::*;
pub use startup::*;

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
    BlockProperties, BlockRole, BoundaryAffinity, CANVAS_STYLE_PROPERTIES,
    CHARACTER_STYLE_PROPERTIES, PARAGRAPH_STYLE_PROPERTIES, CharacterProperties, Color, Document,
    DocumentError, DocumentId, DocumentStyleAssignment, Encoding, FileFormat, FileFormatOrigin,
    FontSize, FontSlant, Format, FormatOperation, FormattedTextError, HardLineQueryError,
    HistorySemanticChangeKind, HistorySemanticSummary, LineSpacing, ModelTransactionError,
    ParagraphAlignment, Revision, ScriptPosition,
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
pub const VIEM_CORE_ABI_VERSION: u32 = 7;

/// Adds paragraph base direction in the request's fixed-layout extension slot
/// and the context-owned cluster contract, plus explicit fragment resource
/// leases so evicting cached shaping can release native draw data.
pub const VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V3: u32 = 3;
/// Current version of the injected text-measurement provider vtable.
pub const VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION: u32 =
    VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V3;

/// Opaque process-local controller/core token. Zero is always invalid.
pub type ViemCoreHandle = u64;

/// Opaque immutable command-turn effect batch. Zero means that a successful
/// turn emitted no host effects and is never a valid owned handle.
pub type ViemEffectBatchHandle = u64;

/// Opaque view identity scoped to one core. Zero is always invalid, and a
/// removed value is never assigned to another view in that core.
pub type ViemViewId = u64;

/// Select encoding in core using supported BOMs, otherwise strict UTF-8, then
/// ISO-8859-1 fallback. Existing nonzero values remain explicit/forced.
pub const VIEM_ENCODING_DETECT: u32 = 0;
pub const VIEM_ENCODING_UTF8: u32 = 1;
pub const VIEM_ENCODING_LATIN1: u32 = 2;
pub const VIEM_ENCODING_UTF16_LE: u32 = 3;
pub const VIEM_ENCODING_UTF16_BE: u32 = 4;

pub const VIEM_FORMAT_PLAIN_TEXT: u32 = 1;
pub const VIEM_FORMAT_MARKDOWN: u32 = 2;
pub const VIEM_FORMAT_RTF: u32 = 4;
pub const VIEM_FORMAT_MARKDOWN_SOURCE: u32 = 5;
pub const VIEM_FORMAT_CODE: u32 = 7;

pub const VIEM_CLIPBOARD_FORMAT_HTML: u32 = 1;
pub const VIEM_CLIPBOARD_FORMAT_RTF: u32 = 2;

/// Detect the line-ending interpretation through the core's shared open
/// policy.
pub const VIEM_FILE_FORMAT_DETECT: u32 = 0;
pub const VIEM_FILE_FORMAT_UNIX: u32 = 1;
pub const VIEM_FILE_FORMAT_DOS: u32 = 2;
pub const VIEM_FILE_FORMAT_MAC: u32 = 3;

pub const VIEM_FILE_FORMAT_ORIGIN_DETECTED: u32 = 1;
pub const VIEM_FILE_FORMAT_ORIGIN_FORCED: u32 = 2;
pub const VIEM_FILE_FORMAT_ORIGIN_DEFAULTED: u32 = 3;

pub const VIEM_HISTORY_ACTION_CATEGORY_NONE: u32 = 0;
pub const VIEM_HISTORY_ACTION_CATEGORY_TEXT: u32 = 1;
pub const VIEM_HISTORY_ACTION_CATEGORY_STYLE: u32 = 2;
pub const VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT: u32 = 3;
pub const VIEM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER: u32 = 4;
pub const VIEM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA: u32 = 6;
pub const VIEM_HISTORY_ACTION_CATEGORY_MIXED: u32 = 7;

pub const VIEM_DOCUMENT_STATE_HAS_BOM: u32 = 1 << 0;
pub const VIEM_DOCUMENT_STATE_CAN_UNDO: u32 = 1 << 1;
pub const VIEM_DOCUMENT_STATE_CAN_REDO: u32 = 1 << 2;
pub const VIEM_DOCUMENT_STATE_IS_DIRTY: u32 = 1 << 3;
pub const VIEM_DOCUMENT_STATE_READ_ONLY: u32 = 1 << 4;
pub const VIEM_DOCUMENT_STATE_RECOVERED: u32 = 1 << 5;

/// Immutable model/history metadata captured in one serial core query.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemDocumentStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
    pub source_byte_count: u64,
    pub encoding: u32,
    pub format: u32,
    pub file_format: u32,
    pub file_format_origin: u32,
    pub undo_action_category: u32,
    pub redo_action_category: u32,
    pub reserved: [u32; 2],
}

pub const VIEM_DOCUMENT_STATE_V1_SIZE: u32 = size_of::<ViemDocumentStateV1>() as u32;

impl Default for ViemDocumentStateV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_DOCUMENT_STATE_V1_SIZE,
            flags: 0,
            document_id: 0,
            document_revision: 0,
            style_sheet_revision: 0,
            source_byte_count: 0,
            encoding: 0,
            format: 0,
            file_format: 0,
            file_format_origin: 0,
            undo_action_category: VIEM_HISTORY_ACTION_CATEGORY_NONE,
            redo_action_category: VIEM_HISTORY_ACTION_CATEGORY_NONE,
            reserved: [0; 2],
        }
    }
}

/// Exact identity of one immutable formatted projection.
///
/// A caller copies this value from [`ViemFormattedSnapshotInfoV1`] into every
/// dependent range, mapping, and point request. Core rejects a request after
/// the document advances rather than applying its numeric positions to a new
/// projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemFormattedSnapshotIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE: u32 =
    size_of::<ViemFormattedSnapshotIdentityV1>() as u32;

/// Constant-time aggregate metadata for the current formatted projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemFormattedSnapshotInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemFormattedSnapshotIdentityV1,
    pub utf8_length: u64,
    pub utf16_length: u64,
    pub hard_line_count: u64,
}

pub const VIEM_FORMATTED_SNAPSHOT_INFO_V1_SIZE: u32 =
    size_of::<ViemFormattedSnapshotInfoV1>() as u32;

/// One scalar-aligned, half-open UTF-8 range in an exact formatted snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemFormattedUtf8RangeV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemFormattedSnapshotIdentityV1,
    pub utf8_start: u64,
    pub utf8_end: u64,
}

pub const VIEM_FORMATTED_UTF8_RANGE_V1_SIZE: u32 = size_of::<ViemFormattedUtf8RangeV1>() as u32;

/// Logical metadata for one grapheme-aligned formatted point.
///
/// Hard-line indices and grapheme columns are zero based. `hard_line_start`
/// and `hard_line_end` delimit content and exclude the following semantic hard
/// break. The point's UTF-16 offset uses the same document-wide origin as its
/// UTF-8 offset.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemFormattedPointInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemFormattedSnapshotIdentityV1,
    pub utf8_offset: u64,
    pub utf16_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
}

pub const VIEM_FORMATTED_POINT_INFO_V1_SIZE: u32 = size_of::<ViemFormattedPointInfoV1>() as u32;

/// Status returned by every fallible ABI operation.
///
/// Discriminants match the constants published in `include/viem_core.h`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViemStatus {
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
    CoreBusy = 20,
    InvalidView = 21,
    InvalidProvider = 22,
    ProviderFailure = 23,
    InvalidKey = 24,
    CoreFailure = 25,
    /// The shaping provider cannot guarantee a stable interior from the
    /// bounded context supplied by core. No partial layout is installed.
    UnstableShapingContext = 26,
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

/// Options for [`viem_core_create`].
///
/// `struct_size` must be at least [`VIEM_DOCUMENT_OPTIONS_SIZE`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemDocumentOptions {
    pub struct_size: u32,
    pub encoding: u32,
    pub format: u32,
    pub file_format: u32,
}

pub const VIEM_DOCUMENT_OPTIONS_SIZE: u32 = std::mem::size_of::<ViemDocumentOptions>() as u32;

impl Default for ViemDocumentOptions {
    fn default() -> Self {
        Self {
            struct_size: VIEM_DOCUMENT_OPTIONS_SIZE,
            encoding: VIEM_ENCODING_UTF8,
            format: VIEM_FORMAT_PLAIN_TEXT,
            file_format: VIEM_FILE_FORMAT_DETECT,
        }
    }
}

pub const VIEM_PROVIDER_THREADING_ANY_WORKER: u32 = 1;
pub const VIEM_PROVIDER_THREADING_DEDICATED_SERIAL: u32 = 2;
pub const VIEM_PROVIDER_THREADING_FRONTEND_MAIN: u32 = 3;

pub const VIEM_RENDER_THREADING_ANY: u32 = 1;
pub const VIEM_RENDER_THREADING_DEDICATED_SERIAL: u32 = 2;
pub const VIEM_RENDER_THREADING_FRONTEND_MAIN: u32 = 3;

pub const VIEM_LAYOUT_EXECUTION_WORKER_POOL: u32 = 1;
pub const VIEM_LAYOUT_EXECUTION_DEDICATED_SERIAL: u32 = 2;
pub const VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN: u32 = 3;

pub const VIEM_TEXT_DIRECTION_AUTO: u32 = 0;
pub const VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT: u32 = 1;
pub const VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT: u32 = 2;

pub const VIEM_FONT_SLANT_UPRIGHT: u32 = 0;
pub const VIEM_FONT_SLANT_ITALIC: u32 = 1;
pub const VIEM_FONT_SLANT_OBLIQUE: u32 = 2;

pub const VIEM_SHAPE_PURPOSE_METRICS_ONLY: u32 = 1;
pub const VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA: u32 = 2;

pub const VIEM_BOUNDARY_AFFINITY_UPSTREAM: u32 = 1;
pub const VIEM_BOUNDARY_AFFINITY_DOWNSTREAM: u32 = 2;

pub const VIEM_KEY_CHARACTER: u32 = 1;
pub const VIEM_KEY_ESCAPE: u32 = 2;
pub const VIEM_KEY_ENTER: u32 = 3;
pub const VIEM_KEY_TAB: u32 = 4;
pub const VIEM_KEY_BACKSPACE: u32 = 5;
pub const VIEM_KEY_DELETE: u32 = 6;
pub const VIEM_KEY_LEFT: u32 = 7;
pub const VIEM_KEY_RIGHT: u32 = 8;
pub const VIEM_KEY_UP: u32 = 9;
pub const VIEM_KEY_DOWN: u32 = 10;
pub const VIEM_KEY_HOME: u32 = 11;
pub const VIEM_KEY_END: u32 = 12;
pub const VIEM_KEY_PAGE_UP: u32 = 13;
pub const VIEM_KEY_PAGE_DOWN: u32 = 14;
pub const VIEM_KEY_CONTROL_CHARACTER: u32 = 15;
pub const VIEM_KEY_BACK_TAB: u32 = 16;
pub const VIEM_KEY_DOCUMENT_START: u32 = 17;
pub const VIEM_KEY_DOCUMENT_END: u32 = 18;
pub const VIEM_KEY_SHIFT_ENTER: u32 = 19;
pub const VIEM_KEY_WORD_LEFT: u32 = 20;
pub const VIEM_KEY_WORD_RIGHT: u32 = 21;
pub const VIEM_KEY_FUNCTION: u32 = 22;
pub const VIEM_KEY_COPY_SELECTION: u32 = 23;
pub const VIEM_KEY_MODIFIER_SHIFT: u32 = 1;
pub const VIEM_KEY_MODIFIER_CONTROL: u32 = 2;
pub const VIEM_KEY_MODIFIER_ALT: u32 = 4;
pub const VIEM_KEY_MODIFIER_COMMAND: u32 = 8;

pub const VIEM_COMMAND_STATUS_NONE: u32 = 0;
pub const VIEM_COMMAND_STATUS_COMPLETE: u32 = 1;
pub const VIEM_COMMAND_STATUS_PENDING: u32 = 2;
pub const VIEM_COMMAND_STATUS_CANCELLED: u32 = 3;
pub const VIEM_COMMAND_STATUS_NEEDS_MORE_LAYOUT: u32 = 4;
pub const VIEM_COMMAND_STATUS_SEARCH_NOT_FOUND: u32 = 5;
pub const VIEM_COMMAND_STATUS_UNSUPPORTED: u32 = 6;
pub const VIEM_COMMAND_STATUS_ERROR: u32 = 7;
pub const VIEM_COMMAND_STATUS_READ_ONLY: u32 = 8;

pub const VIEM_MODE_NORMAL: u32 = 1;
pub const VIEM_MODE_INSERT: u32 = 2;
pub const VIEM_MODE_REPLACE: u32 = 3;
pub const VIEM_MODE_VISUAL_CHARACTER: u32 = 4;
pub const VIEM_MODE_VISUAL_LINE: u32 = 5;
pub const VIEM_MODE_VISUAL_BLOCK: u32 = 6;
pub const VIEM_MODE_COMMAND_LINE: u32 = 7;
pub const VIEM_MODE_SELECTION_CHARACTER: u32 = 11;
pub const VIEM_MODE_SELECTION_LINE: u32 = 12;
pub const VIEM_MODE_SELECTION_BLOCK: u32 = 13;
pub const VIEM_MODE_SELECT_CHARACTER: u32 = 8;
pub const VIEM_MODE_SELECT_LINE: u32 = 9;
pub const VIEM_MODE_SELECT_BLOCK: u32 = 10;
pub const VIEM_SELECTION_ORIGIN_MOUSE: u32 = 1;
pub const VIEM_SELECTION_ORIGIN_KEY: u32 = 2;
pub const VIEM_SELECTION_ORIGIN_COMMAND: u32 = 3;

pub const VIEM_CLIPBOARD_TARGET_CLIPBOARD: u32 = 1;
pub const VIEM_CLIPBOARD_TARGET_PRIMARY: u32 = 2;

pub const VIEM_CLIPBOARD_TURN_HAS_READ: u32 = 1 << 0;
pub const VIEM_CLIPBOARD_TURN_WRITABLE: u32 = 1 << 1;

pub const VIEM_EFFECT_BATCH_HAS_EX_OUTCOME: u32 = 1 << 0;
pub const VIEM_EFFECT_BATCH_EX_DOCUMENT_CHANGED: u32 = 1 << 1;
pub const VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION: u32 = 1 << 2;
pub const VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY: u32 = 1 << 3;

pub const VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER: u32 = 1 << 0;
pub const VIEM_REGISTER_KIND_NONE: u32 = 0;
pub const VIEM_REGISTER_KIND_CHARACTER: u32 = 1;
pub const VIEM_REGISTER_KIND_LINE: u32 = 2;
pub const VIEM_REGISTER_KIND_BLOCK: u32 = 3;

pub const VIEM_EX_FRONTEND_EDIT: u32 = 1;
pub const VIEM_EX_FRONTEND_NEW: u32 = 2;
pub const VIEM_EX_FRONTEND_WRITE: u32 = 3;
pub const VIEM_EX_FRONTEND_SAVE_AS: u32 = 4;
pub const VIEM_EX_FRONTEND_QUIT: u32 = 5;
pub const VIEM_EX_FRONTEND_QUIT_ALL: u32 = 6;
pub const VIEM_EX_FRONTEND_WRITE_QUIT: u32 = 7;
pub const VIEM_EX_FRONTEND_XIT: u32 = 8;
pub const VIEM_EX_FRONTEND_WRITE_ALL: u32 = 9;
pub const VIEM_EX_FRONTEND_MARKS: u32 = 10;
pub const VIEM_EX_FRONTEND_REGISTERS: u32 = 11;
pub const VIEM_EX_FRONTEND_JUMPS: u32 = 12;
pub const VIEM_EX_FRONTEND_OPTIONS: u32 = 13;
pub const VIEM_EX_FRONTEND_PRINT_LINES: u32 = 14;
pub const VIEM_EX_FRONTEND_NORMAL: u32 = 15;
pub const VIEM_EX_FRONTEND_SPLIT: u32 = 16;
pub const VIEM_EX_FRONTEND_MESSAGE: u32 = 17;
pub const VIEM_EX_FRONTEND_EDIT_NEW_WINDOW: u32 = 18;
pub const VIEM_EX_FRONTEND_PWD: u32 = 19;
pub const VIEM_EX_FRONTEND_CD: u32 = 20;
pub const VIEM_EX_FRONTEND_CHECKTIME: u32 = 21;
/// A `CTRL-W` window effect. `window_command` names it and `window_count`
/// carries its count or one-based pane index when `VIEM_EX_FRONTEND_HAS_COUNT`
/// is set.
pub const VIEM_EX_FRONTEND_WINDOW: u32 = 22;
pub const VIEM_EX_FRONTEND_NEW_PANE: u32 = 23;
pub const VIEM_EX_FRONTEND_ARGUMENT: u32 = 24;
pub const VIEM_EX_FRONTEND_READ: u32 = 25;
pub const VIEM_EX_FRONTEND_SOURCE: u32 = 26;
pub const VIEM_EX_FRONTEND_FILE: u32 = 27;
pub const VIEM_EX_FRONTEND_ONLY: u32 = 28;

pub const VIEM_WINDOW_FOCUS_DOWN: u32 = 1;
pub const VIEM_WINDOW_FOCUS_UP: u32 = 2;
pub const VIEM_WINDOW_FOCUS_NEXT: u32 = 3;
pub const VIEM_WINDOW_FOCUS_PREVIOUS: u32 = 4;
pub const VIEM_WINDOW_FOCUS_TOP: u32 = 5;
pub const VIEM_WINDOW_FOCUS_BOTTOM: u32 = 6;
pub const VIEM_WINDOW_FOCUS_LAST_ACCESSED: u32 = 7;
pub const VIEM_WINDOW_ROTATE_DOWN: u32 = 8;
pub const VIEM_WINDOW_ROTATE_UP: u32 = 9;
pub const VIEM_WINDOW_EXCHANGE: u32 = 10;
pub const VIEM_WINDOW_MOVE_TO_TOP: u32 = 11;
pub const VIEM_WINDOW_MOVE_TO_BOTTOM: u32 = 12;
pub const VIEM_WINDOW_CLOSE_OTHERS: u32 = 13;
pub const VIEM_WINDOW_GROW: u32 = 14;
pub const VIEM_WINDOW_SHRINK: u32 = 15;
pub const VIEM_WINDOW_SET_HEIGHT: u32 = 16;
pub const VIEM_WINDOW_EQUALIZE_HEIGHTS: u32 = 17;

pub const VIEM_EX_FRONTEND_FORCE: u32 = 1 << 0;
pub const VIEM_EX_FRONTEND_HAS_PATH: u32 = 1 << 1;
pub const VIEM_EX_FRONTEND_HAS_RANGE: u32 = 1 << 2;
pub const VIEM_EX_FRONTEND_NUMBER: u32 = 1 << 3;
pub const VIEM_EX_FRONTEND_LIST: u32 = 1 << 4;
pub const VIEM_EX_FRONTEND_LITERAL: u32 = 1 << 5;
/// `window_count` carries an explicit count or pane index.
pub const VIEM_EX_FRONTEND_HAS_COUNT: u32 = 1 << 6;
pub const VIEM_EX_FRONTEND_WRITE_FIRST: u32 = 1 << 7;
pub const VIEM_EX_FRONTEND_HAS_LINE: u32 = 1 << 8;

pub const VIEM_EX_OPTION_WRAP: u32 = 1;
pub const VIEM_EX_OPTION_LINEBREAK: u32 = 2;
pub const VIEM_EX_OPTION_FILE_FORMAT: u32 = 3;
pub const VIEM_EX_OPTION_FILE_FORMATS: u32 = 4;
pub const VIEM_EX_OPTION_IGNORECASE: u32 = 5;
pub const VIEM_EX_OPTION_SMARTCASE: u32 = 6;
pub const VIEM_EX_OPTION_WRAPSCAN: u32 = 7;
pub const VIEM_EX_OPTION_TEXTWIDTH: u32 = 8;
pub const VIEM_EX_OPTION_AUTOINDENT: u32 = 9;
pub const VIEM_EX_OPTION_TABSTOP: u32 = 10;
pub const VIEM_EX_OPTION_SHIFTWIDTH: u32 = 11;
pub const VIEM_EX_OPTION_SOFTTABSTOP: u32 = 12;
pub const VIEM_EX_OPTION_EXPANDTAB: u32 = 13;
pub const VIEM_EX_OPTION_SMARTTAB: u32 = 14;
pub const VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_ENTER: u32 = 15;
pub const VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_OPEN_LINE: u32 = 16;

pub const VIEM_EX_OPTION_VALUE_BOOLEAN: u32 = 1;
pub const VIEM_EX_OPTION_VALUE_FILE_FORMAT: u32 = 2;
pub const VIEM_EX_OPTION_VALUE_FILE_FORMATS: u32 = 3;
/// `scalar_value` carries the number.
pub const VIEM_EX_OPTION_VALUE_NUMBER: u32 = 4;
pub const VIEM_EX_OPTION_VALUE_STRING: u32 = 5;
pub const VIEM_EX_OPTION_LIST: u32 = 17;
pub const VIEM_EX_OPTION_LISTCHARS: u32 = 18;
pub const VIEM_EX_OPTION_KEYMODEL: u32 = 21;
pub const VIEM_EX_OPTION_SELECTMODE: u32 = 22;
pub const VIEM_EX_OPTION_AUTOSELECT: u32 = 23;
pub const VIEM_EX_OPTION_HLSEARCH: u32 = 19;
pub const VIEM_EX_OPTION_INCSEARCH: u32 = 20;

pub const VIEM_EX_JUMP_CURRENT: u32 = 1 << 0;

pub const VIEM_OUTCOME_HAS_COMMAND: u32 = 1 << 0;
pub const VIEM_OUTCOME_CURSOR_MOVED: u32 = 1 << 1;
pub const VIEM_OUTCOME_DOCUMENT_CHANGED: u32 = 1 << 2;
pub const VIEM_OUTCOME_MODE_CHANGED: u32 = 1 << 3;
pub const VIEM_OUTCOME_LAYOUT_CHANGED: u32 = 1 << 4;
pub const VIEM_OUTCOME_HAS_POSITION_MAP: u32 = 1 << 5;
pub const VIEM_OUTCOME_HAS_LAYOUT: u32 = 1 << 6;
pub const VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS: u32 = 1 << 7;
pub const VIEM_OUTCOME_HAS_COMPOSITION_CHANGES: u32 = 1 << 8;

/// Length-delimited UTF-8. A null pointer is valid only when `length` is zero.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemUtf8Slice {
    pub data: *const u8,
    pub length: u64,
}

impl Default for ViemUtf8Slice {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            length: 0,
        }
    }
}

/// One clipboard snapshot/capability for an input turn, including an optional
/// validated Viem fragment JSON image. Each target may occur at most once.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemClipboardTurnEntryV2 {
    pub struct_size: u32,
    pub flags: u32,
    pub target: u32,
    pub reserved: u32,
    pub generation: u64,
    pub plain_text: ViemUtf8Slice,
    pub fragment_json: ViemUtf8Slice,
}
pub const VIEM_CLIPBOARD_TURN_ENTRY_V2_SIZE: u32 = size_of::<ViemClipboardTurnEntryV2>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemCommandTurnContextV2 {
    pub struct_size: u32,
    pub reserved: u32,
    pub clipboards: *const ViemClipboardTurnEntryV2,
    pub clipboard_count: u64,
}
pub const VIEM_COMMAND_TURN_CONTEXT_V2_SIZE: u32 = size_of::<ViemCommandTurnContextV2>() as u32;

impl Default for ViemClipboardTurnEntryV2 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_CLIPBOARD_TURN_ENTRY_V2_SIZE,
            flags: 0,
            target: 0,
            reserved: 0,
            generation: 0,
            plain_text: ViemUtf8Slice::default(),
            fragment_json: ViemUtf8Slice::default(),
        }
    }
}

impl Default for ViemCommandTurnContextV2 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_COMMAND_TURN_CONTEXT_V2_SIZE,
            reserved: 0,
            clipboards: std::ptr::null(),
            clipboard_count: 0,
        }
    }
}

/// Offset and length inside an effect batch's copied byte arena. Unless a field
/// explicitly documents otherwise, referenced bytes are valid UTF-8.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemEffectBytesRefV1 {
    pub offset: u64,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemClipboardWriteV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub target: u32,
    pub register_kind: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub plain_text: ViemEffectBytesRefV1,
    pub first_hard_break: u64,
    pub hard_break_count: u64,
}

pub const VIEM_CLIPBOARD_WRITE_V1_SIZE: u32 = size_of::<ViemClipboardWriteV1>() as u32;

/// One displayed option value embedded by an `OPTIONS` frontend request.
/// For `FILE_FORMATS`, `first_file_format..+file_format_count` indexes the
/// copied file-format value array in exact display order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExOptionDisplayV1 {
    pub struct_size: u32,
    pub name: u32,
    pub value_kind: u32,
    pub scalar_value: u32,
    pub first_file_format: u64,
    pub file_format_count: u64,
    pub text: ViemEffectBytesRefV1,
}

pub const VIEM_EX_OPTION_DISPLAY_V1_SIZE: u32 = size_of::<ViemExOptionDisplayV1>() as u32;

/// One resolved mark captured against the effect batch's exact formatted
/// revision. `line_text` contains the complete hard-line content without its
/// semantic break.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExMarkV1 {
    pub struct_size: u32,
    pub name: u32,
    pub utf8_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
    pub line_text: ViemEffectBytesRefV1,
}

pub const VIEM_EX_MARK_V1_SIZE: u32 = size_of::<ViemExMarkV1>() as u32;

/// One resolved Vim register captured for an Ex info request. Hard-break
/// indices reference the shared hard-break array and are UTF-8 byte offsets
/// inside `text`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExRegisterV1 {
    pub struct_size: u32,
    pub name: u32,
    pub register_kind: u32,
    pub reserved: u32,
    pub text: ViemEffectBytesRefV1,
    pub first_hard_break: u64,
    pub hard_break_count: u64,
}

pub const VIEM_EX_REGISTER_V1_SIZE: u32 = size_of::<ViemExRegisterV1>() as u32;

/// One jump captured in oldest-to-newest order. Exactly one record has
/// `CURRENT` when the list is nonempty. Line text excludes the semantic break.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExJumpV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub list_index: u64,
    pub utf8_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
    pub line_text: ViemEffectBytesRefV1,
}

pub const VIEM_EX_JUMP_V1_SIZE: u32 = size_of::<ViemExJumpV1>() as u32;

/// One exact formatted hard line captured for PRINT_LINES. Text excludes the
/// semantic hard break; request NUMBER/LIST flags describe its presentation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExTextLineV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub hard_line_index: u64,
    pub utf8_start: u64,
    pub utf8_end: u64,
    pub text: ViemEffectBytesRefV1,
}

pub const VIEM_EX_TEXT_LINE_V1_SIZE: u32 = size_of::<ViemExTextLineV1>() as u32;

/// One raw Ex host request in command execution order.
///
/// `text` is the optional path, `:normal` command string, or concatenated
/// Unicode mark/register-name scalar sequence according to `kind`. Hard-line
/// ranges are inclusive and zero based. Option indices apply only to OPTIONS.
/// `first_payload..+payload_count` selects the corresponding mark, register,
/// jump, or text-line array for those four info-request kinds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemExFrontendRequestV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub flags: u32,
    /// One `VIEM_WINDOW_*` value when `kind` is `VIEM_EX_FRONTEND_WINDOW`.
    pub window_command: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub text: ViemEffectBytesRefV1,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub first_option: u64,
    pub option_count: u64,
    pub first_payload: u64,
    pub payload_count: u64,
    /// With `VIEM_EX_FRONTEND_HAS_COUNT`: window count/index, or initial row
    /// height for SPLIT and NEW_PANE.
    pub window_count: u64,
    /// One VIEM_ARGUMENT_* command for argument navigation.
    pub argument_command: u32,
    pub reserved: u32,
    pub argument_count: u64,
    /// With HAS_LINE, a one-based initial line; zero means the last line.
    pub argument_line: u64,
}

pub const VIEM_EX_FRONTEND_REQUEST_V1_SIZE: u32 = size_of::<ViemExFrontendRequestV1>() as u32;

/// Exact sizes and command identity for an immutable owned effect batch.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemEffectBatchInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub batch_handle: ViemEffectBatchHandle,
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

pub const VIEM_EFFECT_BATCH_INFO_V1_SIZE: u32 = size_of::<ViemEffectBatchInfoV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemTextMetricsV1 {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemShapedBoundsV1 {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemOpenTypeFeatureV1 {
    pub tag: [u8; 4],
    pub value: u32,
}

/// ABI-v1 resolved shaping style supplied to the frontend callback.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemResolvedTextStyleV1 {
    pub struct_size: u32,
    pub slant: u32,
    pub direction: u32,
    pub has_language: u32,
    pub has_script: u32,
    pub reserved: u32,
    pub size: f32,
    pub weight: f32,
    pub letter_spacing: f32,
    pub script_position: u32,
    pub font_families: *const ViemUtf8Slice,
    pub font_family_count: u64,
    pub language: ViemUtf8Slice,
    pub script: ViemUtf8Slice,
    pub features: *const ViemOpenTypeFeatureV1,
    pub feature_count: u64,
}

pub const VIEM_RESOLVED_TEXT_STYLE_V1_SIZE: u32 = size_of::<ViemResolvedTextStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemShapeStyleRunV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub style: ViemResolvedTextStyleV1,
}

pub const VIEM_SHAPE_STYLE_RUN_V1_SIZE: u32 = size_of::<ViemShapeStyleRunV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Provider-owned opaque render resource.
///
/// `identifier` is an integer token, never a native pointer for core to
/// dereference. Core only compares, caches, and transports the value. The
/// provider pins callback response resources until the next shape call. Core
/// retains one independent lease per returned fragment, shared by its caches
/// and snapshots. The token remains usable while leased and its generation is
/// current. Borrowed frontend exports must not outlive their exact layout.
/// Releasing a lease is valid even after its generation or view is retired.
pub struct ViemRenderRunHandleV1 {
    pub owner: u64,
    pub identifier: u64,
    pub metrics_generation: u64,
    pub threading: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemClusterCaretStopV1 {
    pub text_offset: u64,
    pub inline_offset: f32,
    pub affinity: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemShapedClusterV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub advance: f32,
    pub metrics: ViemTextMetricsV1,
    pub typographic_bounds: ViemShapedBoundsV1,
    pub ink_bounds: ViemShapedBoundsV1,
    pub bidi_level: u32,
    pub has_render_run: u32,
    pub fallback_font: ViemUtf8Slice,
    pub caret_stops: *const ViemClusterCaretStopV1,
    pub caret_stop_count: u64,
    pub render_run: ViemRenderRunHandleV1,
}

pub const VIEM_SHAPED_CLUSTER_V1_SIZE: u32 = size_of::<ViemShapedClusterV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemShapingDiagnosticV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub message: ViemUtf8Slice,
}

pub const VIEM_SHAPING_DIAGNOSTIC_V1_SIZE: u32 = size_of::<ViemShapingDiagnosticV1>() as u32;

/// One immutable shaping request. Every pointer is borrowed only for the
/// synchronous `shape_batch` callback. `text_start` and
/// `text_end` delimit the stable ownership interior represented by `text`.
/// Shape `context_before + text + context_after`, and return each whole cluster
/// whose logical start lies in that interior. Such a cluster may end in the
/// following context; a cluster beginning in preceding context is omitted.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemShapeRequestV1 {
    pub struct_size: u32,
    pub purpose: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub text: ViemUtf8Slice,
    pub context_before: ViemUtf8Slice,
    pub context_after: ViemUtf8Slice,
    pub style_runs: *const ViemShapeStyleRunV1,
    pub style_run_count: u64,
    pub default_style: ViemResolvedTextStyleV1,
    pub scale: f32,
    pub has_render_run_policy: u32,
    pub render_run_owner: u64,
    pub render_run_threading: u32,
    /// The containing paragraph's `VIEM_TEXT_DIRECTION_*` value.
    pub paragraph_base_direction: u32,
}

pub const VIEM_SHAPE_REQUEST_V1_SIZE: u32 = size_of::<ViemShapeRequestV1>() as u32;

/// One callback-owned shaping response. All pointed-to arrays and UTF-8 bytes
/// must remain readable when the callback returns and until the next provider
/// callback for this view; core copies them immediately. Response `text_start`
/// and `text_end` echo the request's ownership interior, not the union of the
/// returned cluster ranges.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemShapeResponseV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub clusters: *const ViemShapedClusterV1,
    pub cluster_count: u64,
    pub visual_order: *const u64,
    pub visual_order_count: u64,
    pub default_metrics: ViemTextMetricsV1,
    pub diagnostics: *const ViemShapingDiagnosticV1,
    pub diagnostic_count: u64,
}

impl Default for ViemShapeResponseV1 {
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
            default_metrics: ViemTextMetricsV1::default(),
            diagnostics: std::ptr::null(),
            diagnostic_count: 0,
        }
    }
}

pub const VIEM_SHAPE_RESPONSE_V1_SIZE: u32 = size_of::<ViemShapeResponseV1>() as u32;

/// Retain the resource identifiers in one shaped fragment. The returned opaque
/// lease has its own lifetime and must remain releasable after view removal.
pub type ViemRetainRenderRunsCallback = unsafe extern "C" fn(
    context: *mut c_void, handles: *const ViemRenderRunHandleV1, count: u64,
) -> *mut c_void;
/// May be invoked on any thread. The provider marshals native destruction to
/// its required executor. This callback must never call back into the core.
pub type ViemReleaseRenderRunsCallback = unsafe extern "C" fn(lease: *mut c_void);

pub type ViemMetricsGenerationCallback = unsafe extern "C" fn(context: *mut c_void) -> u64;
pub type ViemShapeBatchCallback = unsafe extern "C" fn(
    context: *mut c_void,
    requests: *const ViemShapeRequestV1,
    request_count: u64,
    responses: *mut ViemShapeResponseV1,
    response_capacity: u64,
) -> u32;

/// Versioned frontend-owned provider table. Core copies this prefix while
/// adding a view; `context` and callback-owned response storage must remain
/// valid until that view is removed or its core is successfully destroyed. A
/// destroy call returning [`ViemStatus::CoreBusy`] has not destroyed the core
/// and does not end that lifetime. Render-run tokens have the separate
/// lease and generation lifetime documented on [`ViemRenderRunHandleV1`]. A
/// successful callback affirms stable ownership interiors. A provider
/// unable to make that bounded-context guarantee returns
/// [`ViemStatus::UnstableShapingContext`]; core caches and installs none of that
/// batch.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemTextMeasurementProviderV1 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub context: *mut c_void,
    pub measurement_environment_id: u64,
    pub threading: u32,
    pub has_render_run_policy: u32,
    pub render_run_owner: u64,
    pub render_run_threading: u32,
    pub reserved: u32,
    pub metrics_generation: Option<ViemMetricsGenerationCallback>,
    pub shape_batch: Option<ViemShapeBatchCallback>,
    pub retain_render_runs: Option<ViemRetainRenderRunsCallback>,
    pub release_render_runs: Option<ViemReleaseRenderRunsCallback>,
}

pub const VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE: u32 =
    size_of::<ViemTextMeasurementProviderV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViemViewOptionsV1 {
    pub struct_size: u32,
    pub execution_context: u32,
    pub width: f32,
    pub height: f32,
    pub padding_top: f32,
    pub padding_left: f32,
    pub padding_bottom: f32,
    pub padding_right: f32,
}

pub const VIEM_VIEW_OPTIONS_V1_SIZE: u32 = size_of::<ViemViewOptionsV1>() as u32;

impl Default for ViemViewOptionsV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_VIEW_OPTIONS_V1_SIZE,
            execution_context: VIEM_LAYOUT_EXECUTION_WORKER_POOL,
            width: 800.0,
            height: 600.0,
            padding_top: 0.0,
            padding_left: 0.0,
            padding_bottom: 0.0,
            padding_right: 0.0,
        }
    }
}

/// Request an absolute per-view presentation origin. `left` is always
/// requested. With `VIEM_VIEWPORT_ORIGIN_HAS_TOP`, the complete expected
/// identity must match the current immutable layout before core atomically
/// installs bounded regional layout and both requested coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViemViewportOriginV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    /// Exact state identity from `ViemViewportStateV1`. These fields are
    /// ignored for horizontal-only requests.
    pub expected_document_id: u64,
    pub expected_document_revision: u64,
    pub expected_layout_revision: u64,
    pub expected_configuration_generation: u64,
    pub expected_measurement_environment_id: u64,
    pub expected_metrics_generation: u64,
}

pub const VIEM_VIEWPORT_ORIGIN_V1_SIZE: u32 = size_of::<ViemViewportOriginV1>() as u32;
pub const VIEM_VIEWPORT_ORIGIN_HAS_TOP: u32 = 1 << 0;

impl Default for ViemViewportOriginV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_VIEWPORT_ORIGIN_V1_SIZE,
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

pub const VIEM_VIEWPORT_STATE_WRAP: u32 = 1 << 0;
pub const VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT: u32 = 1 << 1;
pub const VIEM_VIEWPORT_STATE_TOP_EXACT: u32 = 1 << 2;
pub const VIEM_VIEWPORT_STATE_HAS_LAYOUT: u32 = 1 << 3;
pub const VIEM_VIEWPORT_STATE_LINEBREAK: u32 = 1 << 4;
pub const VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT: u32 = 1 << 5;

/// Current presentation origin and the exact dependency identity observed in
/// the same serial query. `maximum_left` describes visible rows only; without
/// its exact flag it is a provisional lower bound, not an authoritative clamp.
/// `maximum_top` includes document padding and final-row geometry. Its exact
/// flag proves the document end in the current coordinate system; prefix
/// heights may remain estimated. Without the flag it is a scrollbar estimate,
/// not an authoritative clamp.
/// `scale` is always the exact positive view-local magnification. A
/// missing exact top flag means the installed snapshot uses an estimated
/// prefix; the value remains the view's current coordinate but must not be
/// treated as a durable absolute document position.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViemViewportStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    pub maximum_left: f32,
    pub maximum_top: f32,
    pub scale: f32,
    pub document_id: u64,
    pub document_revision: u64,
    pub layout_revision: u64,
    pub configuration_generation: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
}

pub const VIEM_VIEWPORT_STATE_V1_SIZE: u32 = size_of::<ViemViewportStateV1>() as u32;

impl Default for ViemViewportStateV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_VIEWPORT_STATE_V1_SIZE,
            flags: 0,
            left: 0.0,
            top: 0.0,
            maximum_left: 0.0,
            maximum_top: 0.0,
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
/// this value from [`ViemLayoutSnapshotInfoV1`] into subsequent geometry
/// requests; core never silently substitutes a newer snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemLayoutSnapshotIdentityV1 {
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

pub const VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE: u32 =
    size_of::<ViemLayoutSnapshotIdentityV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutInsetsV1 {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutRectV1 {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Normalized RGBA components copied from the core style model. Every
/// component is finite and in the inclusive range zero to one.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemRgbaV1 {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

pub const VIEM_STYLE_NAMESPACE_BLOCK: u32 = 1;
pub const VIEM_STYLE_NAMESPACE_CHARACTER: u32 = 2;

pub const VIEM_STYLE_ROLE_NONE: u32 = 0;
pub const VIEM_STYLE_ROLE_DOCUMENT: u32 = 1;
pub const VIEM_STYLE_ROLE_PARAGRAPH: u32 = 2;
pub const VIEM_STYLE_ROLE_QUOTE: u32 = 3;
pub const VIEM_STYLE_ROLE_CODE_BLOCK: u32 = 4;
pub const VIEM_STYLE_ROLE_LIST: u32 = 5;
pub const VIEM_STYLE_ROLE_LIST_ITEM: u32 = 6;

pub const VIEM_STYLE_ORIGIN_SOURCE_BACKED: u32 = 1;
pub const VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION: u32 = 2;
pub const VIEM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY: u32 = 3;

pub const VIEM_STYLE_DEFINITION_HAS_PARENT: u32 = 1 << 0;
pub const VIEM_STYLE_DEFINITION_HAS_NEXT_STYLE: u32 = 1 << 1;
pub const VIEM_STYLE_DEFINITION_BASE_PARAGRAPH: u32 = 1 << 3;
pub const VIEM_STYLE_DEFINITION_INTERNAL: u32 = 1 << 5;
pub const VIEM_STYLE_DEFINITION_INTERNAL_LIST: u32 = 1 << 6;
/// A generated Code syntax definition that has not been edited or persisted.
pub const VIEM_STYLE_DEFINITION_IMPLICIT: u32 = 1 << 7;

pub const VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS: u32 = 1 << 0;
pub const VIEM_STYLE_CAPABILITY_EDIT_PARENT: u32 = 1 << 1;
pub const VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE: u32 = 1 << 2;
pub const VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME: u32 = 1 << 3;
pub const VIEM_STYLE_CAPABILITY_ASSIGN: u32 = 1 << 4;
pub const VIEM_STYLE_CAPABILITY_DELETE: u32 = 1 << 5;

pub const VIEM_STYLE_PROPERTY_CANVAS_BACKGROUND: u32 = 1;
pub const VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP: u32 = 2;
pub const VIEM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT: u32 = 3;
pub const VIEM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM: u32 = 4;
pub const VIEM_STYLE_PROPERTY_CANVAS_PADDING_LEFT: u32 = 5;
pub const VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP: u32 = 6;
pub const VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM: u32 = 7;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING: u32 = 8;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT: u32 = 9;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT: u32 = 10;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT: u32 = 11;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT: u32 = 12;
pub const VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION: u32 = 13;
pub const VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES: u32 = 14;
pub const VIEM_STYLE_PROPERTY_CHARACTER_SIZE: u32 = 15;
pub const VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT: u32 = 16;
pub const VIEM_STYLE_PROPERTY_CHARACTER_BOLD: u32 = 27;
pub const VIEM_STYLE_PROPERTY_CHARACTER_SLANT: u32 = 17;
pub const VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND: u32 = 18;
pub const VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND: u32 = 19;
pub const VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE: u32 = 20;
pub const VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH: u32 = 21;
pub const VIEM_STYLE_PROPERTY_CHARACTER_LANGUAGE: u32 = 22;
pub const VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION: u32 = 23;
pub const VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES: u32 = 24;
pub const VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING: u32 = 25;
pub const VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION: u32 = 26;
pub const VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT: u32 = 28;
pub const VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT: u32 = 29;
pub const VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP: u32 = 30;
pub const VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT: u32 = 31;
pub const VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM: u32 = 32;
pub const VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT: u32 = 33;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH: u32 = 34;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR: u32 = 35;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH: u32 = 36;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR: u32 = 37;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH: u32 = 38;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR: u32 = 39;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH: u32 = 40;
pub const VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR: u32 = 41;
pub const VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND: u32 = 42;

pub const VIEM_STYLE_VALUE_NONE: u32 = 0;
pub const VIEM_STYLE_VALUE_FLOAT: u32 = 1;
pub const VIEM_STYLE_VALUE_UNSIGNED: u32 = 2;
pub const VIEM_STYLE_VALUE_BOOLEAN: u32 = 3;
pub const VIEM_STYLE_VALUE_COLOR: u32 = 4;
pub const VIEM_STYLE_VALUE_STRING: u32 = 5;
pub const VIEM_STYLE_VALUE_STRING_LIST: u32 = 6;
pub const VIEM_STYLE_VALUE_FONT_SLANT: u32 = 7;
pub const VIEM_STYLE_VALUE_WRITING_DIRECTION: u32 = 8;
pub const VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES: u32 = 9;
pub const VIEM_STYLE_VALUE_LINE_SPACING: u32 = 10;
pub const VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT: u32 = 11;
pub const VIEM_STYLE_VALUE_SCRIPT_POSITION: u32 = 12;
pub const VIEM_STYLE_VALUE_PERCENTAGE: u32 = 13;
pub const VIEM_SCRIPT_POSITION_NORMAL: u32 = 0;
pub const VIEM_SCRIPT_POSITION_SUPERSCRIPT: u32 = 1;
pub const VIEM_SCRIPT_POSITION_SUBSCRIPT: u32 = 2;

pub const VIEM_STYLE_VALUE_ITEM_STRING: u32 = 1;
pub const VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE: u32 = 2;

pub const VIEM_STYLE_LINE_SPACING_NORMAL: u32 = 1;
pub const VIEM_STYLE_LINE_SPACING_MULTIPLIER: u32 = 2;
pub const VIEM_STYLE_LINE_SPACING_AT_LEAST: u32 = 3;
pub const VIEM_STYLE_LINE_SPACING_EXACT: u32 = 4;

pub const VIEM_STYLE_PARAGRAPH_ALIGNMENT_START: u32 = 1;
pub const VIEM_STYLE_PARAGRAPH_ALIGNMENT_END: u32 = 2;
pub const VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER: u32 = 3;

pub const VIEM_STYLE_PROPERTY_DECLARED: u32 = 1 << 0;
pub const VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT: u32 = 1 << 1;
pub const VIEM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE: u32 = 1 << 2;

pub const VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY: u32 = 1;
pub const VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE: u32 = 2;
pub const VIEM_STYLE_CONTRIBUTOR_CHARACTER_STYLE: u32 = 3;
pub const VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS: u32 = 4;
pub const VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER: u32 = 5;
pub const VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH: u32 = 6;
pub const VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER: u32 = 7;
pub const VIEM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER: u32 = 8;

pub const VIEM_STYLE_EDIT_SET_DECLARATION: u32 = 1;
pub const VIEM_STYLE_EDIT_CLEAR_DECLARATION: u32 = 2;
pub const VIEM_STYLE_EDIT_SET_PARENT: u32 = 3;
pub const VIEM_STYLE_EDIT_CLEAR_PARENT: u32 = 4;
pub const VIEM_STYLE_EDIT_SET_NEXT_STYLE: u32 = 5;
pub const VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE: u32 = 6;
pub const VIEM_STYLE_EDIT_SET_DISPLAY_NAME: u32 = 7;

/// Exact identity of one immutable normalized style sheet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleSheetIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
}

pub const VIEM_STYLE_SHEET_IDENTITY_V1_SIZE: u32 = size_of::<ViemStyleSheetIdentityV1>() as u32;

/// Byte range in the UTF-8 arena returned with one style-sheet export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleStringRefV1 {
    pub offset: u64,
    pub length: u64,
}

/// Fixed summary and exact array/arena sizes for one immutable style sheet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleSheetInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemStyleSheetIdentityV1,
    pub definition_count: u64,
    pub property_count: u64,
    pub value_item_count: u64,
    pub dependency_count: u64,
    pub string_bytes: u64,
}

pub const VIEM_STYLE_SHEET_INFO_V1_SIZE: u32 = size_of::<ViemStyleSheetInfoV1>() as u32;

/// Tagged style value. String and array payloads index the arena and value-item
/// array copied by the same atomic export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemStyleValueV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub enum_value: u32,
    pub reserved: u32,
    pub number: f32,
    pub number_reserved: f32,
    pub color: ViemRgbaV1,
    pub string: ViemStyleStringRefV1,
    pub first_item: u64,
    pub item_count: u64,
}

pub const VIEM_STYLE_VALUE_V1_SIZE: u32 = size_of::<ViemStyleValueV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleValueItemV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub string: ViemStyleStringRefV1,
    pub unsigned_value: u32,
    pub reserved: u32,
}

pub const VIEM_STYLE_VALUE_ITEM_V1_SIZE: u32 = size_of::<ViemStyleValueItemV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleDependencyV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub style_id: ViemStyleStringRefV1,
}

pub const VIEM_STYLE_DEPENDENCY_V1_SIZE: u32 = size_of::<ViemStyleDependencyV1>() as u32;

/// One definition in namespace/stable-ID order. Property indices refer to the
/// property array from the same export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemStyleDefinitionV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub namespace: u32,
    pub role: u32,
    pub origin: u32,
    pub capabilities: u32,
    pub stable_id: ViemStyleStringRefV1,
    pub display_name: ViemStyleStringRefV1,
    pub parent_id: ViemStyleStringRefV1,
    pub next_style_id: ViemStyleStringRefV1,
    pub first_property: u64,
    pub property_count: u64,
}

pub const VIEM_STYLE_DEFINITION_V1_SIZE: u32 = size_of::<ViemStyleDefinitionV1>() as u32;

/// One applicable schema property for a definition. `declared` is meaningful
/// only with `DECLARED`; an absent optional effective value uses kind NONE.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemStylePropertyV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub property: u32,
    pub contributor_kind: u32,
    pub contributor_namespace: u32,
    pub reserved: u32,
    pub declared: ViemStyleValueV1,
    pub effective: ViemStyleValueV1,
    pub contributor_style_id: ViemStyleStringRefV1,
    pub first_dependency: u64,
    pub dependency_count: u64,
}

pub const VIEM_STYLE_PROPERTY_V1_SIZE: u32 = size_of::<ViemStylePropertyV1>() as u32;

/// Caller-owned item for an array-valued style edit. `text` contains a font
/// family or four-byte OpenType tag according to `kind`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemStyleEditValueItemV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub text: ViemUtf8Slice,
    pub unsigned_value: u32,
    pub reserved: u32,
}

pub const VIEM_STYLE_EDIT_VALUE_ITEM_V1_SIZE: u32 = size_of::<ViemStyleEditValueItemV1>() as u32;

impl Default for ViemStyleEditValueItemV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_STYLE_EDIT_VALUE_ITEM_V1_SIZE,
            kind: 0,
            text: ViemUtf8Slice::default(),
            unsigned_value: 0,
            reserved: 0,
        }
    }
}

/// Caller-owned typed value for one style field edit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemStyleEditValueV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub enum_value: u32,
    pub reserved: u32,
    pub number: f32,
    pub number_reserved: f32,
    pub color: ViemRgbaV1,
    pub text: ViemUtf8Slice,
    pub items: *const ViemStyleEditValueItemV1,
    pub item_count: u64,
}

pub const VIEM_STYLE_EDIT_VALUE_V1_SIZE: u32 = size_of::<ViemStyleEditValueV1>() as u32;

impl Default for ViemStyleEditValueV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_STYLE_EDIT_VALUE_V1_SIZE,
            kind: VIEM_STYLE_VALUE_NONE,
            enum_value: 0,
            reserved: 0,
            number: 0.0,
            number_reserved: 0.0,
            color: ViemRgbaV1::default(),
            text: ViemUtf8Slice::default(),
            items: std::ptr::null(),
            item_count: 0,
        }
    }
}

/// Exact-revision request to edit one field of an existing editable style.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemStyleEditV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: ViemStyleSheetIdentityV1,
    pub namespace: u32,
    pub operation: u32,
    pub property: u32,
    pub reserved: u32,
    pub style_id: ViemUtf8Slice,
    pub value: ViemStyleEditValueV1,
}

pub const VIEM_STYLE_EDIT_V1_SIZE: u32 = size_of::<ViemStyleEditV1>() as u32;

impl Default for ViemStyleEditV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_STYLE_EDIT_V1_SIZE,
            flags: 0,
            identity: ViemStyleSheetIdentityV1::default(),
            namespace: 0,
            operation: 0,
            property: 0,
            reserved: 0,
            style_id: ViemUtf8Slice::default(),
            value: ViemStyleEditValueV1::default(),
        }
    }
}

/// Immutable capability for one explicit live style-edit group. `token` is a
/// process-wide non-reused identifier; all remaining identity fields are part
/// of the capability and must be passed back unchanged. The begin revisions
/// intentionally do not advance as grouped edits commit newer snapshots.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemStyleEditGroupV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub token: u64,
    pub view_id: u64,
    pub document_id: u64,
    pub begin_document_revision: u64,
    pub begin_style_sheet_revision: u64,
}

pub const VIEM_STYLE_EDIT_GROUP_V1_SIZE: u32 = size_of::<ViemStyleEditGroupV1>() as u32;

impl Default for ViemStyleEditGroupV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_STYLE_EDIT_GROUP_V1_SIZE,
            flags: 0,
            token: 0,
            view_id: 0,
            document_id: 0,
            begin_document_revision: 0,
            begin_style_sheet_revision: 0,
        }
    }
}

impl ViemStyleEditGroupV1 {
    fn from_core(group: StyleEditGroup) -> Self {
        Self {
            struct_size: VIEM_STYLE_EDIT_GROUP_V1_SIZE,
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

pub const VIEM_TEXT_PAINT_HAS_BACKGROUND: u32 = 1 << 0;
pub const VIEM_TEXT_PAINT_UNDERLINE: u32 = 1 << 1;
pub const VIEM_TEXT_PAINT_STRIKETHROUGH: u32 = 1 << 2;
pub const VIEM_TEXT_PAINT_DEFAULT_FOREGROUND: u32 = 1 << 3;
pub const VIEM_LAYOUT_PAINT_DEFAULT_CANVAS: u32 = 1 << 0;

/// Fully resolved paint-only text attributes. Foreground is always present;
/// background is meaningful only with `HAS_BACKGROUND`. Decoration flags mean
/// that the corresponding decoration is enabled.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemTextPaintV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub foreground: ViemRgbaV1,
    pub background: ViemRgbaV1,
}

pub const VIEM_TEXT_PAINT_V1_SIZE: u32 = size_of::<ViemTextPaintV1>() as u32;

/// Fixed canvas/default-paint state and required override-run count from one
/// exact immutable layout snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutPaintInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub canvas_background: ViemRgbaV1,
    pub default_paint: ViemTextPaintV1,
    pub paint_run_count: u64,
}

pub const VIEM_LAYOUT_PAINT_INFO_V1_SIZE: u32 = size_of::<ViemLayoutPaintInfoV1>() as u32;

/// One logical half-open UTF-8 range whose resolved paint differs from the
/// default paint. Runs are ordered in document order and never overlap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemPaintStyleRunV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub paint: ViemTextPaintV1,
}

pub const VIEM_PAINT_STYLE_RUN_V1_SIZE: u32 = size_of::<ViemPaintStyleRunV1>() as u32;

pub const VIEM_LAYOUT_SNAPSHOT_FULL_DOCUMENT: u32 = 1 << 0;
pub const VIEM_LAYOUT_SNAPSHOT_PREFIX_EXACT: u32 = 1 << 1;
pub const VIEM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT: u32 = 1 << 2;
pub const VIEM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT: u32 = 1 << 3;

/// Fixed summary and required array lengths for a layout export. Geometry is
/// in document-layout coordinates; presentation applies the viewport origin
/// separately. Partial snapshots report their exact materialized coverage.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutSnapshotInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub usable_width: f32,
    pub content_width: f32,
    pub total_height: f32,
    pub content_insets: ViemLayoutInsetsV1,
    pub coverage_hard_line_start: u64,
    pub coverage_hard_line_end: u64,
    pub document_hard_line_count: u64,
    pub coverage_y_start: f32,
    pub coverage_y_end: f32,
    pub row_count: u64,
    pub cluster_count: u64,
    pub caret_count: u64,
}

pub const VIEM_LAYOUT_SNAPSHOT_INFO_V1_SIZE: u32 = size_of::<ViemLayoutSnapshotInfoV1>() as u32;

pub const VIEM_VISUAL_ROW_HAS_PARAGRAPH: u32 = 1 << 0;
pub const VIEM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS: u32 = 1 << 1;
pub const VIEM_VISUAL_ROW_WRAPS_TO_NEXT: u32 = 1 << 2;

/// One positioned visual row. `first_*` and `*_count` index the arrays copied
/// by the same atomic snapshot export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemVisualRowV1 {
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

pub const VIEM_VISUAL_ROW_V1_SIZE: u32 = size_of::<ViemVisualRowV1>() as u32;

pub const VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN: u32 = 1 << 0;

/// One shaped cluster in row visual order. The optional render-run token keeps
/// the provider ownership and generation lifetime declared on
/// [`ViemRenderRunHandleV1`]; core never transfers or retains it for callers.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemPositionedClusterV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub row_index: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub x: f32,
    pub advance: f32,
    pub typographic_bounds: ViemLayoutRectV1,
    pub ink_bounds: ViemLayoutRectV1,
    pub bidi_level: u32,
    pub reserved: u32,
    pub render_run: ViemRenderRunHandleV1,
}

pub const VIEM_POSITIONED_CLUSTER_V1_SIZE: u32 = size_of::<ViemPositionedClusterV1>() as u32;

/// Noneditable layout furniture, with label-local bytes in a separate export
/// blob. These offsets never address formatted document text.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutDecorationV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub row_index: u64,
    pub label_byte_start: u64,
    pub label_byte_length: u64,
    pub x: f32,
    pub advance: f32,
    pub font_size: f32,
    pub reserved: f32,
    pub typographic_bounds: ViemLayoutRectV1,
    pub ink_bounds: ViemLayoutRectV1,
    pub render_run: ViemRenderRunHandleV1,
    pub paint: ViemTextPaintV1,
}
pub const VIEM_LAYOUT_DECORATION_V1_SIZE: u32 = size_of::<ViemLayoutDecorationV1>() as u32;
pub const VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER: u32 = 1 << 1;
pub const VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND: u32 = 1 << 2;
pub const VIEM_LAYOUT_DECORATION_BLOCK_BORDER: u32 = 1 << 3;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutDecorationsInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub decoration_count: u64,
    pub label_bytes: u64,
}
pub const VIEM_LAYOUT_DECORATIONS_INFO_V1_SIZE: u32 =
    size_of::<ViemLayoutDecorationsInfoV1>() as u32;

/// One legal shaping caret stop positioned in document-layout coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemPositionedCaretV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub row_index: u64,
    pub text_offset: u64,
    pub x: f32,
    pub reserved: f32,
}

pub const VIEM_POSITIONED_CARET_V1_SIZE: u32 = size_of::<ViemPositionedCaretV1>() as u32;

/// Revision-bound request for logical endpoint geometry. The result may be a
/// containing-cluster fallback without changing the logical endpoint.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutCaretRequestV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub text_offset: u64,
}

pub const VIEM_LAYOUT_CARET_REQUEST_V1_SIZE: u32 = size_of::<ViemLayoutCaretRequestV1>() as u32;

/// Revision-bound request for hit testing a document-layout coordinate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutHitTestRequestV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub x: f32,
    pub y: f32,
}

pub const VIEM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE: u32 =
    size_of::<ViemLayoutHitTestRequestV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemLayoutCaretPointV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub layout_revision: u64,
    pub text_offset: u64,
}

pub const VIEM_LAYOUT_CARET_POINT_V1_SIZE: u32 = size_of::<ViemLayoutCaretPointV1>() as u32;

pub const VIEM_CARET_GEOMETRY_CLUSTER_FALLBACK: u32 = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLayoutCaretGeometryV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub point: ViemLayoutCaretPointV1,
    pub rect: ViemLayoutRectV1,
    pub row_index: u64,
}

pub const VIEM_LAYOUT_CARET_GEOMETRY_V1_SIZE: u32 = size_of::<ViemLayoutCaretGeometryV1>() as u32;

pub const VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR: u32 = 1 << 0;
pub const VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT: u32 = 1 << 1;
pub const VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK: u32 = 1 << 2;
pub const VIEM_VIEW_PRESENTATION_HAS_COMMAND_LINE: u32 = 1 << 3;
pub const VIEM_VIEW_PRESENTATION_HAS_DESIRED_X: u32 = 1 << 4;
/// Route the next input to the core before native editing shortcuts.
pub const VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING: u32 = 1 << 5;
/// Route prompt register selectors to core, including native text events.
pub const VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING: u32 = 1 << 6;

/// Current controller presentation state. Linear Visual anchors do not retain
/// a visual affinity, so their affinity field is zero unless the exact flag is
/// present. Visual Block endpoints retain exact layout affinity and x edges.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemViewPresentationV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub mode: u32,
    /// Row disambiguation for `caret_utf8_start` when the caret shape is a
    /// boundary. It is not a character selector: never use it to choose which
    /// character a cell caret covers.
    pub cursor_affinity: u32,
    pub visual_anchor_affinity: u32,
    /// `VIEM_CARET_SHAPE_CELL` or `VIEM_CARET_SHAPE_BOUNDARY`.
    pub caret_shape: u32,
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
    /// Exact formatted range the caret occupies. A cell covers one grapheme of
    /// hard-line content; a boundary is empty, with both ends at the caret.
    pub caret_utf8_start: u64,
    pub caret_utf8_end: u64,
}

pub const VIEM_VIEW_PRESENTATION_V1_SIZE: u32 = size_of::<ViemViewPresentationV1>() as u32;

/// The caret covers one grapheme. Draw the character cell `caret_utf8_start
/// ..caret_utf8_end`; boundary affinity does not apply.
pub const VIEM_CARET_SHAPE_CELL: u32 = 1;
/// The caret sits between graphemes at `caret_utf8_start`, with
/// `cursor_affinity` choosing its row at a soft-wrap boundary.
pub const VIEM_CARET_SHAPE_BOUNDARY: u32 = 2;

pub const VIEM_COMMAND_LINE_KIND_NONE: u32 = 0;
pub const VIEM_COMMAND_LINE_KIND_EX: u32 = 1;
pub const VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD: u32 = 2;
pub const VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD: u32 = 3;

/// Exact identity of one command-line byte export. The opaque state identity
/// changes whenever the exported kind, cursor, or UTF-8 bytes change, even
/// when the document revision does not.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCommandLineIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub state_identity: [u8; 32],
}

pub const VIEM_COMMAND_LINE_IDENTITY_V1_SIZE: u32 = size_of::<ViemCommandLineIdentityV1>() as u32;

/// Fixed summary and required byte length for one command-line export. The
/// cursor is a UTF-8 byte boundary in the copied bytes; the prompt prefix is
/// represented by `identity.kind` and is not part of those bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCommandLineInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemCommandLineIdentityV1,
    pub utf8_length: u64,
    pub cursor_utf8_offset: u64,
}

pub const VIEM_COMMAND_LINE_INFO_V1_SIZE: u32 = size_of::<ViemCommandLineInfoV1>() as u32;

pub const VIEM_VISUAL_SELECTION_KIND_NONE: u32 = 0;
pub const VIEM_VISUAL_SELECTION_KIND_CHARACTER: u32 = 1;
pub const VIEM_VISUAL_SELECTION_KIND_LINE: u32 = 2;
pub const VIEM_VISUAL_SELECTION_KIND_BLOCK: u32 = 3;

pub const VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW: u32 = 1 << 0;
pub const VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE: u32 = 1 << 1;
pub const VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES: u32 = 1 << 2;

/// Exact identity of one Visual-selection export. Layout identity makes every
/// rectangle revision-bound; the opaque state identity additionally changes
/// when the selection payload changes without relayout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemVisualSelectionIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub layout: ViemLayoutSnapshotIdentityV1,
    pub state_identity: [u8; 32],
}

pub const VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE: u32 =
    size_of::<ViemVisualSelectionIdentityV1>() as u32;

/// Required array counts for the exact current Visual selection. Segments are
/// ordered by logical UTF-8 document order; rectangles are ordered by visual
/// row and x coordinate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemVisualSelectionInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemVisualSelectionIdentityV1,
    pub segment_count: u64,
    pub rectangle_count: u64,
}

pub const VIEM_VISUAL_SELECTION_INFO_V1_SIZE: u32 = size_of::<ViemVisualSelectionInfoV1>() as u32;

/// One exact logical half-open UTF-8 selection segment. Character- and
/// Linewise Visual selections have one untagged segment. Blockwise segments
/// retain their originating row, hard line, and display-edge affinities.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemVisualSelectionSegmentV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub row_index: u64,
    pub hard_line_index: u64,
    pub left_affinity: u32,
    pub right_affinity: u32,
}

pub const VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE: u32 =
    size_of::<ViemVisualSelectionSegmentV1>() as u32;

/// One drawable rectangle for a logical selection segment. `segment_index`
/// indexes the segment array returned by the same atomic copy.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemVisualSelectionRectangleV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub row_index: u64,
    pub segment_index: u64,
    pub rect: ViemLayoutRectV1,
}

pub const VIEM_VISUAL_SELECTION_RECTANGLE_V1_SIZE: u32 =
    size_of::<ViemVisualSelectionRectangleV1>() as u32;

pub const VIEM_LOGICAL_SELECTION_KIND_NONE: u32 = 0;
pub const VIEM_LOGICAL_SELECTION_KIND_CHARACTER: u32 = 1;
pub const VIEM_LOGICAL_SELECTION_KIND_LINE: u32 = 2;
pub const VIEM_LOGICAL_SELECTION_KIND_BLOCK: u32 = 3;

pub const VIEM_SEMANTIC_STYLE_STRONG: u32 = 1;
pub const VIEM_SEMANTIC_STYLE_EMPHASIS: u32 = 2;

pub const VIEM_SEMANTIC_STYLE_STATE_OFF: u32 = 0;
pub const VIEM_SEMANTIC_STYLE_STATE_ON: u32 = 1;
pub const VIEM_SEMANTIC_STYLE_STATE_MIXED: u32 = 2;

pub const VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE: u32 = 1 << 0;
/// Exact Insert/Replace caret target; toggles change pending typing policy only.
pub const VIEM_SEMANTIC_STYLE_TYPING_CONTEXT: u32 = 1 << 3;
pub const VIEM_SEMANTIC_STYLE_CAN_SET: u32 = 1 << 1;
pub const VIEM_SEMANTIC_STYLE_CAN_CLEAR: u32 = 1 << 2;

/// Exact current logical selection identity without layout geometry. Only
/// Character- and Linewise identities name an actionable contiguous range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemLogicalSelectionIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub state_identity: [u8; 32],
}

pub const VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE: u32 =
    size_of::<ViemLogicalSelectionIdentityV1>() as u32;

/// Check/mixed state and exact source-adapter capabilities for one semantic
/// style at the current core-owned selection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSemanticStylePresentationV1 {
    pub struct_size: u32,
    pub style: u32,
    pub state: u32,
    pub flags: u32,
    pub selection: ViemLogicalSelectionIdentityV1,
}

pub const VIEM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE: u32 =
    size_of::<ViemSemanticStylePresentationV1>() as u32;

/// Revision- and selection-bound request to set or clear one semantic style.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSetSemanticStyleV1 {
    pub struct_size: u32,
    pub style: u32,
    pub enabled: u32,
    pub reserved: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}

pub const VIEM_SET_SEMANTIC_STYLE_V1_SIZE: u32 = size_of::<ViemSetSemanticStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemDirectStyleEditV1 {
    pub struct_size: u32,
    pub operation: u32,
    pub property: u32,
    pub reserved: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
    pub value: ViemStyleEditValueV1,
}
pub const VIEM_DIRECT_STYLE_EDIT_V1_SIZE: u32 = size_of::<ViemDirectStyleEditV1>() as u32;

pub const VIEM_PLACE_CURSOR_EXTEND_SELECTION: u32 = 1 << 0;

/// Revision-bound pointer-placement intention. `text_offset` is a formatted
/// UTF-8 boundary returned by exact hit testing; affinity preserves the visual
/// side at wrap or bidi split carets.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemPlaceCursorV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_revision: u64,
    pub text_offset: u64,
    pub affinity: u32,
    pub reserved: u32,
}

pub const VIEM_PLACE_CURSOR_V1_SIZE: u32 = size_of::<ViemPlaceCursorV1>() as u32;

/// Exact model identity for a native file-format change. Only the concrete
/// Unix, DOS, and Mac values are accepted; detection is an open-time policy.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemSetFileFormatV1 {
    pub struct_size: u32,
    pub file_format: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const VIEM_SET_FILE_FORMAT_V1_SIZE: u32 = size_of::<ViemSetFileFormatV1>() as u32;

pub const VIEM_FORMAT_OPERATION_REINTERPRET: u32 = 0;
pub const VIEM_FORMAT_OPERATION_CONVERT: u32 = 1;

/// Explicit format operation bound to one exact document snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSetFormatV1 {
    pub struct_size: u32,
    pub format: u32,
    pub operation: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
}
pub const VIEM_SET_FORMAT_V1_SIZE: u32 = size_of::<ViemSetFormatV1>() as u32;

/// Lossless source transcoding request; automatic detection is not a target.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSetEncodingV1 {
    pub struct_size: u32,
    pub encoding: u32,
    pub document_id: u64,
    pub document_revision: u64,
}
pub const VIEM_SET_ENCODING_V1_SIZE: u32 = size_of::<ViemSetEncodingV1>() as u32;

pub const VIEM_LIST_STYLE_NONE: u32 = 0;
pub const VIEM_LIST_STYLE_BULLET: u32 = 1;
pub const VIEM_LIST_STYLE_NUMBERED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSetListStyleV1 {
    pub struct_size: u32,
    pub style: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}
pub const VIEM_SET_LIST_STYLE_V1_SIZE: u32 = size_of::<ViemSetListStyleV1>() as u32;

pub const VIEM_LIST_CAN_INDENT: u32 = 1;
pub const VIEM_LIST_CAN_UNINDENT: u32 = 2;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemListIndentV1 {
    pub struct_size: u32,
    pub unindent: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}
pub const VIEM_LIST_INDENT_V1_SIZE: u32 = size_of::<ViemListIndentV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemSetParagraphStyleV1 {
    pub struct_size: u32,
    /// Zero means Base Paragraph; one through six select a heading.
    pub level: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}
pub const VIEM_SET_PARAGRAPH_STYLE_V1_SIZE: u32 = size_of::<ViemSetParagraphStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemAssignStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: ViemStyleSheetIdentityV1,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
    pub style_id: ViemUtf8Slice,
}
pub const VIEM_ASSIGN_STYLE_V1_SIZE: u32 = size_of::<ViemAssignStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemCreateStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: ViemStyleSheetIdentityV1,
    pub style_id: ViemUtf8Slice,
    pub display_name: ViemUtf8Slice,
    pub parent_id: ViemUtf8Slice,
    pub next_style_id: ViemUtf8Slice,
}
pub const VIEM_CREATE_STYLE_V1_SIZE: u32 = size_of::<ViemCreateStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemDeleteStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: ViemStyleSheetIdentityV1,
    pub style_id: ViemUtf8Slice,
}
pub const VIEM_DELETE_STYLE_V1_SIZE: u32 = size_of::<ViemDeleteStyleV1>() as u32;

impl Default for ViemSetFileFormatV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_SET_FILE_FORMAT_V1_SIZE,
            file_format: VIEM_FILE_FORMAT_UNIX,
            document_id: 0,
            document_revision: 0,
        }
    }
}

/// Exact acknowledgement that the current authoritative source snapshot was
/// written successfully by a native document frontend.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemMarkSavedV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const VIEM_MARK_SAVED_V1_SIZE: u32 = size_of::<ViemMarkSavedV1>() as u32;

impl Default for ViemMarkSavedV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_MARK_SAVED_V1_SIZE,
            reserved: 0,
            document_id: 0,
            document_revision: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemKeyInputV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub codepoint: u32,
    /// Function-key modifiers; zero for other key kinds.
    pub modifiers: u32,
}

pub const VIEM_KEY_INPUT_V1_SIZE: u32 = size_of::<ViemKeyInputV1>() as u32;

/// Exact formatted-snapshot replacement target for beginning native marked
/// text. The frontend supplies its current selection, or an empty range at its
/// insertion caret, in UTF-8 byte offsets for `document_revision`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemCompositionBeginV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub replacement_start: u64,
    pub replacement_end: u64,
}

pub const VIEM_COMPOSITION_BEGIN_V1_SIZE: u32 = size_of::<ViemCompositionBeginV1>() as u32;

/// One replacement of the temporary marked-text overlay. Selection offsets
/// are relative to `marked_text` and are UTF-8 byte offsets.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemCompositionUpdateV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub marked_text: ViemUtf8Slice,
    pub selected_start: u64,
    pub selected_end: u64,
}

pub const VIEM_COMPOSITION_UPDATE_V1_SIZE: u32 = size_of::<ViemCompositionUpdateV1>() as u32;

/// Final UTF-8 payload to install as one authoritative document transaction
/// and one undo unit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemCompositionCommitV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub committed_text: ViemUtf8Slice,
}

pub const VIEM_COMPOSITION_COMMIT_V1_SIZE: u32 = size_of::<ViemCompositionCommitV1>() as u32;

/// Revision precondition for explicitly cancelling marked text.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemCompositionCancelV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
}

pub const VIEM_COMPOSITION_CANCEL_V1_SIZE: u32 = size_of::<ViemCompositionCancelV1>() as u32;

pub const VIEM_COMPOSITION_OVERLAY_ACTIVE: u32 = 1 << 0;

/// Exact identity of one disposable per-view composition projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCompositionOverlayIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub generation: u64,
}

pub const VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE: u32 =
    size_of::<ViemCompositionOverlayIdentityV1>() as u32;

/// Metadata and exact ranges in the temporary composed UTF-8 projection.
/// An inactive view returns a zeroed value without `ACTIVE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCompositionOverlayInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: ViemCompositionOverlayIdentityV1,
    pub utf8_length: u64,
    pub replacement_start: u64,
    pub replacement_end: u64,
    pub marked_start: u64,
    pub marked_end: u64,
    pub selected_start: u64,
    pub selected_end: u64,
}

pub const VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE: u32 =
    size_of::<ViemCompositionOverlayInfoV1>() as u32;

/// Scalar-aligned range read from one exact composition projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCompositionOverlayUtf8RangeV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: ViemCompositionOverlayIdentityV1,
    pub start: u64,
    pub end: u64,
}

pub const VIEM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE: u32 =
    size_of::<ViemCompositionOverlayUtf8RangeV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemCoreOutcomeV1 {
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

pub const VIEM_CORE_OUTCOME_V1_SIZE: u32 = size_of::<ViemCoreOutcomeV1>() as u32;

impl Default for ViemCoreOutcomeV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_CORE_OUTCOME_V1_SIZE,
            command_status: VIEM_COMMAND_STATUS_NONE,
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

fn ffi_boundary(operation: impl FnOnce() -> Result<(), ViemStatus>) -> ViemStatus {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => ViemStatus::Ok,
        Ok(Err(status)) => status,
        Err(_) => ViemStatus::Panic,
    }
}

fn parse_encoding(raw: u32) -> Result<Option<Encoding>, ViemStatus> {
    match raw {
        VIEM_ENCODING_DETECT => Ok(None),
        VIEM_ENCODING_UTF8 => Ok(Some(Encoding::Utf8)),
        VIEM_ENCODING_LATIN1 => Ok(Some(Encoding::Latin1)),
        VIEM_ENCODING_UTF16_LE => Ok(Some(Encoding::Utf16Le)),
        VIEM_ENCODING_UTF16_BE => Ok(Some(Encoding::Utf16Be)),
        _ => Err(ViemStatus::InvalidEncoding),
    }
}

fn parse_format(raw: u32) -> Result<Format, ViemStatus> {
    match raw {
        VIEM_FORMAT_PLAIN_TEXT => Ok(Format::PlainText),
        VIEM_FORMAT_MARKDOWN => Ok(Format::Markdown),
        VIEM_FORMAT_RTF => Ok(Format::Rtf),
        VIEM_FORMAT_MARKDOWN_SOURCE => Ok(Format::MarkdownSource),
        VIEM_FORMAT_CODE => Ok(Format::Code),
        _ => Err(ViemStatus::InvalidFormat),
    }
}

fn parse_file_format(raw: u32) -> Result<Option<FileFormat>, ViemStatus> {
    match raw {
        VIEM_FILE_FORMAT_DETECT => Ok(None),
        VIEM_FILE_FORMAT_UNIX => Ok(Some(FileFormat::Unix)),
        VIEM_FILE_FORMAT_DOS => Ok(Some(FileFormat::Dos)),
        VIEM_FILE_FORMAT_MAC => Ok(Some(FileFormat::Mac)),
        _ => Err(ViemStatus::InvalidFileFormat),
    }
}

fn parse_concrete_file_format(raw: u32) -> Result<FileFormat, ViemStatus> {
    parse_file_format(raw)?.ok_or(ViemStatus::InvalidFileFormat)
}

fn encoding_to_ffi(encoding: Encoding) -> u32 {
    match encoding {
        Encoding::Utf8 => VIEM_ENCODING_UTF8,
        Encoding::Latin1 => VIEM_ENCODING_LATIN1,
        Encoding::Utf16Le => VIEM_ENCODING_UTF16_LE,
        Encoding::Utf16Be => VIEM_ENCODING_UTF16_BE,
    }
}

fn format_to_ffi(format: Format) -> u32 {
    match format {
        Format::PlainText => VIEM_FORMAT_PLAIN_TEXT,
        Format::Markdown => VIEM_FORMAT_MARKDOWN,

        Format::Rtf => VIEM_FORMAT_RTF,
        Format::MarkdownSource => VIEM_FORMAT_MARKDOWN_SOURCE,

        Format::Code => VIEM_FORMAT_CODE,
    }
}

fn file_format_to_ffi(file_format: FileFormat) -> u32 {
    match file_format {
        FileFormat::Unix => VIEM_FILE_FORMAT_UNIX,
        FileFormat::Dos => VIEM_FILE_FORMAT_DOS,
        FileFormat::Mac => VIEM_FILE_FORMAT_MAC,
    }
}

fn file_format_origin_to_ffi(origin: FileFormatOrigin) -> u32 {
    match origin {
        FileFormatOrigin::Detected => VIEM_FILE_FORMAT_ORIGIN_DETECTED,
        FileFormatOrigin::Forced => VIEM_FILE_FORMAT_ORIGIN_FORCED,
        FileFormatOrigin::Defaulted => VIEM_FILE_FORMAT_ORIGIN_DEFAULTED,
    }
}

fn history_change_to_ffi(change: HistorySemanticChangeKind) -> u32 {
    match change {
        HistorySemanticChangeKind::Text => VIEM_HISTORY_ACTION_CATEGORY_TEXT,
        HistorySemanticChangeKind::Style => VIEM_HISTORY_ACTION_CATEGORY_STYLE,
        HistorySemanticChangeKind::FileFormat => VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT,
        HistorySemanticChangeKind::HardLineTransfer => {
            VIEM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER
        }
        HistorySemanticChangeKind::SourceMetadata => VIEM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA,
    }
}

fn history_action_category(summary: Option<&HistorySemanticSummary>) -> u32 {
    let Some(changes) = summary.map(HistorySemanticSummary::changes) else {
        return VIEM_HISTORY_ACTION_CATEGORY_NONE;
    };
    let Some(first) = changes.first().copied() else {
        return VIEM_HISTORY_ACTION_CATEGORY_NONE;
    };
    if changes.iter().all(|change| *change == first) {
        history_change_to_ffi(first)
    } else {
        VIEM_HISTORY_ACTION_CATEGORY_MIXED
    }
}

fn checked_length(length: u64) -> Result<usize, ViemStatus> {
    usize::try_from(length)
        .ok()
        .filter(|length| *length <= isize::MAX as usize)
        .ok_or(ViemStatus::LengthOverflow)
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
unsafe fn input_bytes<'a>(pointer: *const u8, length: u64) -> Result<&'a [u8], ViemStatus> {
    let length = checked_length(length)?;
    if length == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(ViemStatus::NullPointer);
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
/// `ViemDocumentOptions` value for the duration of this call.
unsafe fn read_options(
    pointer: *const ViemDocumentOptions,
) -> Result<ViemDocumentOptions, ViemStatus> {
    if pointer.is_null() {
        return Err(ViemStatus::NullPointer);
    }
    if (pointer as usize) % align_of::<ViemDocumentOptions>() != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    // SAFETY: The caller contract guarantees a readable, aligned value and
    // the null case was rejected above. The C representation makes this a
    // field-for-field copy with no Rust-owned resources.
    let options = unsafe { pointer.read() };
    if options.struct_size < VIEM_DOCUMENT_OPTIONS_SIZE {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(options)
}

fn validate_revision(document: &Document, expected: u64) -> Result<(), ViemStatus> {
    if document.revision() == Revision(expected) {
        Ok(())
    } else {
        Err(ViemStatus::StaleRevision)
    }
}

fn formatted_snapshot_identity(document: &Document) -> ViemFormattedSnapshotIdentityV1 {
    ViemFormattedSnapshotIdentityV1 {
        struct_size: VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE,
        reserved: 0,
        document_id: document.id().0,
        document_revision: document.revision().0,
    }
}

fn validate_formatted_snapshot_identity(
    expected: ViemFormattedSnapshotIdentityV1,
    document: &Document,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    let actual = formatted_snapshot_identity(document);
    (expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn formatted_text_status(error: FormattedTextError) -> ViemStatus {
    match error {
        FormattedTextError::InvalidByteOffset { .. }
        | FormattedTextError::InvalidRange { .. }
        | FormattedTextError::InvalidUtf16Offset { .. }
        | FormattedTextError::HardLineOutOfBounds { .. } => ViemStatus::InvalidRange,
        FormattedTextError::NotCharBoundary(_) => ViemStatus::InvalidUtf8Boundary,
        FormattedTextError::NotUtf16Boundary(_) => ViemStatus::InvalidUtf16Boundary,
        FormattedTextError::NotGraphemeBoundary(_) => ViemStatus::NotGraphemeBoundary,
        FormattedTextError::OverlappingSplices { .. } => ViemStatus::InvalidArgument,
        FormattedTextError::ArithmeticOverflow => ViemStatus::LengthOverflow,
        FormattedTextError::LeafIdentityExhausted => ViemStatus::ResourceExhausted,
        FormattedTextError::UnicodeBoundaryResolutionFailed => ViemStatus::CoreFailure,
        FormattedTextError::ResultTextMismatch => ViemStatus::VerificationFailed,
    }
}

fn hard_line_query_status(error: HardLineQueryError) -> ViemStatus {
    match error {
        HardLineQueryError::InvalidLineRange { .. }
        | HardLineQueryError::FormattedOffsetOutOfBounds { .. } => ViemStatus::InvalidRange,
        HardLineQueryError::NotCharacterBoundary { .. } => ViemStatus::InvalidUtf8Boundary,
    }
}

fn formatted_snapshot_info(document: &Document) -> Result<ViemFormattedSnapshotInfoV1, ViemStatus> {
    let snapshot = document.hard_line_snapshot();
    Ok(ViemFormattedSnapshotInfoV1 {
        struct_size: VIEM_FORMATTED_SNAPSHOT_INFO_V1_SIZE,
        reserved: 0,
        identity: formatted_snapshot_identity(document),
        utf8_length: checked_export_count(snapshot.text_length())?,
        utf16_length: checked_export_count(snapshot.utf16_length())?,
        hard_line_count: checked_export_count(snapshot.line_count())?,
    })
}

fn document_status(error: DocumentError) -> ViemStatus {
    match error {
        DocumentError::InvalidRange { .. } => ViemStatus::InvalidRange,
        DocumentError::NotGraphemeBoundary(_) => ViemStatus::NotGraphemeBoundary,
        DocumentError::WrongSnapshot { .. } => ViemStatus::StaleRevision,
        DocumentError::WrongDocument => ViemStatus::InvalidArgument,
        DocumentError::InvalidEncoding { .. }
        | DocumentError::MismatchedBom
        | DocumentError::UnsupportedBom(_) => ViemStatus::InvalidEncoding,
        DocumentError::UnrepresentableCharacter { .. } => ViemStatus::UnrepresentableCharacter,
        DocumentError::AmbiguousProjection => ViemStatus::AmbiguousProjection,
        DocumentError::VerificationFailed
        | DocumentError::FormattedPayloadCannotReproject
        | DocumentError::HardLineTransferProjectionMismatch => ViemStatus::VerificationFailed,
        DocumentError::LineEndingConversionWouldReinterpretContent
        | DocumentError::UnrepresentableFormattedCharacter { .. } => ViemStatus::PolicyRequired,
        DocumentError::UnsupportedFormatting | DocumentError::OpaqueDecodingConflict { .. } => {
            ViemStatus::UnsupportedOperation
        }
        DocumentError::DocumentIdentityExhausted | DocumentError::BlockIdentityExhausted => {
            ViemStatus::ResourceExhausted
        }
        DocumentError::OverlappingEdits
        | DocumentError::OverlappingFormatting
        | DocumentError::InvalidHardLineTransferRange { .. }
        | DocumentError::InvalidHardLineTransferDestination { .. }
        | DocumentError::HardLineTransferDestinationInsideSource { .. } => ViemStatus::InvalidArgument,
        DocumentError::FormattedTextStorage(_) => ViemStatus::InternalError,
    }
}

#[derive(Clone, Copy)]
struct CTextMeasurementProvider {
    context: usize,
    measurement_environment_id: MeasurementEnvironmentId,
    threading: ProviderThreading,
    render_run_policy: Option<RenderRunPolicy>,
    metrics_generation_callback: ViemMetricsGenerationCallback,
    shape_batch_callback: ViemShapeBatchCallback,
    retain_render_runs_callback: Option<ViemRetainRenderRunsCallback>,
    release_render_runs_callback: Option<ViemReleaseRenderRunsCallback>,
}

impl CTextMeasurementProvider {
    /// Copy and validate the current provider table. The frontend retains ownership
    /// of the context and every resource referenced by callbacks.
    unsafe fn from_ffi(provider: *const ViemTextMeasurementProviderV1) -> Result<Self, ViemStatus> {
        if provider.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        if (provider as usize) % align_of::<ViemTextMeasurementProviderV1>() != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        // SAFETY: The API contract requires a readable aligned provider table. Null
        // and alignment were checked before making the field-for-field copy.
        let provider = unsafe { provider.read() };
        if provider.struct_size < VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE
            || provider.abi_version != VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION
            || provider.reserved != 0
        {
            return Err(ViemStatus::InvalidProvider);
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
            .ok_or(ViemStatus::InvalidProvider)?;
        let shape_batch_callback = provider.shape_batch.ok_or(ViemStatus::InvalidProvider)?;
        if render_run_policy.is_some()
            && (provider.retain_render_runs.is_none() || provider.release_render_runs.is_none()) {
            return Err(ViemStatus::InvalidProvider);
        }
        Ok(Self {
            context: provider.context as usize,
            measurement_environment_id: MeasurementEnvironmentId(
                provider.measurement_environment_id,
            ),
            threading,
            render_run_policy,
            metrics_generation_callback,
            shape_batch_callback,
            retain_render_runs_callback: provider.retain_render_runs,
            release_render_runs_callback: provider.release_render_runs,
        })
    }

    fn context(self) -> *mut c_void {
        self.context as *mut c_void
    }
}

struct CRenderResourceLease {
    context: usize,
    release: ViemReleaseRenderRunsCallback,
}
impl Drop for CRenderResourceLease {
    fn drop(&mut self) {
        // SAFETY: The provider transfers one independent lease to core and
        // accepts its exactly-once release from any thread, even after detach.
        unsafe { (self.release)(self.context as *mut c_void) };
    }
}

struct MarshalledStyle {
    _font_families: Vec<ViemUtf8Slice>,
    _features: Vec<ViemOpenTypeFeatureV1>,
    ffi: ViemResolvedTextStyleV1,
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
            .map(|feature| ViemOpenTypeFeatureV1 {
                tag: feature.tag,
                value: feature.value,
            })
            .collect();
        let ffi = ViemResolvedTextStyleV1 {
            struct_size: VIEM_RESOLVED_TEXT_STYLE_V1_SIZE,
            slant: match style.slant {
                FontSlant::Upright => VIEM_FONT_SLANT_UPRIGHT,
                FontSlant::Italic => VIEM_FONT_SLANT_ITALIC,
                FontSlant::Oblique => VIEM_FONT_SLANT_OBLIQUE,
            },
            direction: match style.direction {
                TextDirection::Auto => VIEM_TEXT_DIRECTION_AUTO,
                TextDirection::LeftToRight => VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                TextDirection::RightToLeft => VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT,
            },
            has_language: u32::from(style.language.is_some()),
            has_script: u32::from(style.script.is_some()),
            reserved: u32::from(style.relative_bold),
            size: style.size,
            weight: style.weight,
            letter_spacing: style.letter_spacing,
            script_position: style.script_position as u32,
            font_families: slice_pointer(&font_families),
            font_family_count: font_families.len() as u64,
            language: style
                .language
                .as_deref()
                .map_or_else(ViemUtf8Slice::default, ffi_utf8_slice),
            script: style
                .script
                .as_deref()
                .map_or_else(ViemUtf8Slice::default, ffi_utf8_slice),
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
    style_runs: Vec<ViemShapeStyleRunV1>,
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
            .map(|(run, style)| ViemShapeStyleRunV1 {
                struct_size: VIEM_SHAPE_STYLE_RUN_V1_SIZE,
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

    fn request(&self, request: &ShapeRequest<'_>) -> ViemShapeRequestV1 {
        let (has_render_run_policy, render_run_owner, render_run_threading) =
            request.render_run_policy.map_or((0, 0, 0), |policy| {
                (1, policy.owner.0, ffi_render_threading(policy.threading))
            });
        ViemShapeRequestV1 {
            struct_size: VIEM_SHAPE_REQUEST_V1_SIZE,
            purpose: match request.purpose {
                ShapePurpose::MetricsOnly => VIEM_SHAPE_PURPOSE_METRICS_ONLY,
                ShapePurpose::MetricsAndRenderData => VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA,
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
            paragraph_base_direction: match request.paragraph_base_direction {
                TextDirection::Auto => VIEM_TEXT_DIRECTION_AUTO,
                TextDirection::LeftToRight => VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                TextDirection::RightToLeft => VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT,
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
            .map(|(request, storage)| storage.request(request))
            .collect();
        let mut ffi_responses = vec![ViemShapeResponseV1::default(); requests.len()];
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
        if callback_status == ViemStatus::UnstableShapingContext as u32 {
            return Err(MeasurementError::UnstableShapingContext(
                "C provider declined the bounded shaping context".to_owned(),
            ));
        }
        if callback_status != ViemStatus::Ok as u32 {
            return Err(MeasurementError::Provider(format!(
                "C provider callback returned status {callback_status}"
            )));
        }
        ffi_responses
            .iter()
            .map(|response| {
                // SAFETY: Successful callbacks guarantee that every returned
                // pointer/count pair remains readable while core copies it.
                let mut fragment = unsafe { shaped_fragment_from_ffi(response) }?;
                let handles: Vec<_> = fragment.clusters.iter().filter_map(|cluster|
                    cluster.render_run.as_ref().map(render_run_to_ffi)).collect();
                if !handles.is_empty() {
                    let retain = self.retain_render_runs_callback.ok_or_else(||
                        measurement_failure("render resources require a retain callback"))?;
                    let release = self.release_render_runs_callback.ok_or_else(||
                        measurement_failure("render resources require a release callback"))?;
                    // SAFETY: Handles have been copied and validated, and provider
                    // response resources remain pinned until the next shape call.
                    let context = unsafe { retain(self.context(), handles.as_ptr(), handles.len() as u64) };
                    if context.is_null() { return Err(measurement_failure("provider could not retain render resources")); }
                    let lease: Arc<dyn Send + Sync> = Arc::new(CRenderResourceLease { context: context as usize, release });
                    for handle in fragment.clusters.iter_mut().filter_map(|cluster|cluster.render_run.as_mut()) {
                        handle.retain_resource(Arc::clone(&lease));
                    }
                }
                Ok(fragment)
            })
            .collect()
    }
}

fn ffi_utf8_slice(value: &str) -> ViemUtf8Slice {
    ViemUtf8Slice {
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
        RenderRunThreading::AnyThread => VIEM_RENDER_THREADING_ANY,
        RenderRunThreading::DedicatedSerialExecutor => VIEM_RENDER_THREADING_DEDICATED_SERIAL,
        RenderRunThreading::FrontendMainThread => VIEM_RENDER_THREADING_FRONTEND_MAIN,
    }
}

fn parse_provider_threading(raw: u32) -> Result<ProviderThreading, ViemStatus> {
    match raw {
        VIEM_PROVIDER_THREADING_ANY_WORKER => Ok(ProviderThreading::AnyWorker),
        VIEM_PROVIDER_THREADING_DEDICATED_SERIAL => Ok(ProviderThreading::DedicatedSerialExecutor),
        VIEM_PROVIDER_THREADING_FRONTEND_MAIN => Ok(ProviderThreading::FrontendMainThread),
        _ => Err(ViemStatus::InvalidProvider),
    }
}

fn parse_render_threading(raw: u32) -> Result<RenderRunThreading, ViemStatus> {
    match raw {
        VIEM_RENDER_THREADING_ANY => Ok(RenderRunThreading::AnyThread),
        VIEM_RENDER_THREADING_DEDICATED_SERIAL => Ok(RenderRunThreading::DedicatedSerialExecutor),
        VIEM_RENDER_THREADING_FRONTEND_MAIN => Ok(RenderRunThreading::FrontendMainThread),
        _ => Err(ViemStatus::InvalidProvider),
    }
}

fn parse_ffi_bool(raw: u32) -> Result<bool, ViemStatus> {
    match raw {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ViemStatus::InvalidArgument),
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
    value: ViemUtf8Slice,
    field: &'static str,
) -> Result<String, MeasurementError> {
    let bytes = unsafe { provider_slice(value.data, value.length, field)? };
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| measurement_failure(format!("{field} is not valid UTF-8")))
}

fn text_metrics_from_ffi(metrics: ViemTextMetricsV1) -> TextMetrics {
    TextMetrics {
        ascent: metrics.ascent,
        descent: metrics.descent,
        leading: metrics.leading,
    }
}

fn shaped_bounds_from_ffi(bounds: ViemShapedBoundsV1) -> ShapedBounds {
    ShapedBounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn affinity_from_ffi(raw: u32) -> Result<BoundaryAffinity, MeasurementError> {
    match raw {
        VIEM_BOUNDARY_AFFINITY_UPSTREAM => Ok(BoundaryAffinity::Upstream),
        VIEM_BOUNDARY_AFFINITY_DOWNSTREAM => Ok(BoundaryAffinity::Downstream),
        _ => Err(measurement_failure("invalid caret affinity")),
    }
}

fn render_threading_from_response(raw: u32) -> Result<RenderRunThreading, MeasurementError> {
    match raw {
        VIEM_RENDER_THREADING_ANY => Ok(RenderRunThreading::AnyThread),
        VIEM_RENDER_THREADING_DEDICATED_SERIAL => Ok(RenderRunThreading::DedicatedSerialExecutor),
        VIEM_RENDER_THREADING_FRONTEND_MAIN => Ok(RenderRunThreading::FrontendMainThread),
        _ => Err(measurement_failure("invalid render-run threading value")),
    }
}

unsafe fn shaped_fragment_from_ffi(
    response: &ViemShapeResponseV1,
) -> Result<ShapedFragment, MeasurementError> {
    if response.struct_size < VIEM_SHAPE_RESPONSE_V1_SIZE || response.reserved != 0 {
        return Err(measurement_failure("shaping response prefix is too small"));
    }
    let mut fonts = std::collections::BTreeMap::new();
    let clusters = unsafe {
        provider_slice(
            response.clusters,
            response.cluster_count,
            "response clusters",
        )?
    }
    .iter()
    .map(|cluster| unsafe { shaped_cluster_from_ffi(cluster, &mut fonts) })
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
    cluster: &ViemShapedClusterV1,
    fonts: &mut std::collections::BTreeMap<String, Arc<str>>,
) -> Result<ShapedCluster, MeasurementError> {
    if cluster.struct_size < VIEM_SHAPED_CLUSTER_V1_SIZE || cluster.reserved != 0 {
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
            Ok(RenderRunHandle::new(
                RenderRunOwner(cluster.render_run.owner), cluster.render_run.identifier,
                MetricsGeneration(cluster.render_run.metrics_generation),
                render_threading_from_response(cluster.render_run.threading)?,
            ))
        })
        .transpose()?;
    let bidi_level = u8::try_from(cluster.bidi_level)
        .map_err(|_| measurement_failure("bidi level exceeds u8"))?;
    let font = unsafe { provider_utf8(cluster.fallback_font, "fallback font")? };
    let fallback_font = Arc::clone(fonts.entry(font).or_insert_with_key(|name| Arc::from(name.as_str())));
    Ok(ShapedCluster {
        text_range: checked_response_offset(cluster.text_start)?
            ..checked_response_offset(cluster.text_end)?,
        advance: cluster.advance,
        metrics: text_metrics_from_ffi(cluster.metrics),
        typographic_bounds: shaped_bounds_from_ffi(cluster.typographic_bounds),
        ink_bounds: shaped_bounds_from_ffi(cluster.ink_bounds),
        bidi_level,
        fallback_font,
        caret_stops,
        render_run,
    })
}

unsafe fn shaping_diagnostic_from_ffi(
    diagnostic: &ViemShapingDiagnosticV1,
) -> Result<ShapingDiagnostic, MeasurementError> {
    if diagnostic.struct_size < VIEM_SHAPING_DIAGNOSTIC_V1_SIZE || diagnostic.reserved != 0 {
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
            | ExFrontendRequest::Window(_)
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
        if let Some(ex) = command.ex_outcome.as_mut() {
            let options = ex.option_effects.iter().filter(|effect| matches!(effect.name, ExOptionName::KeyModel | ExOptionName::SelectMode | ExOptionName::AutoSelect))
                .map(|effect| crate::command::ex_execute::ExOptionDisplay { name: effect.name.clone(), value: effect.new_value.clone() }).collect::<Vec<_>>();
            if !options.is_empty() { ex.frontend_requests.push(ExFrontendRequest::Info(ExInfoRequest::Options(options))); }
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
    info: ViemEffectBatchInfoV1,
    clipboard_writes: Vec<ViemClipboardWriteV1>,
    ex_requests: Vec<ViemExFrontendRequestV1>,
    ex_options: Vec<ViemExOptionDisplayV1>,
    ex_marks: Vec<ViemExMarkV1>,
    ex_registers: Vec<ViemExRegisterV1>,
    ex_jumps: Vec<ViemExJumpV1>,
    ex_text_lines: Vec<ViemExTextLineV1>,
    file_formats: Vec<u32>,
    hard_breaks: Vec<u64>,
    strings: Vec<u8>,
}

fn push_effect_text(strings: &mut Vec<u8>, text: &str) -> Result<ViemEffectBytesRefV1, ViemStatus> {
    let offset = checked_export_count(strings.len())?;
    let length = checked_export_count(text.len())?;
    strings.extend_from_slice(text.as_bytes());
    Ok(ViemEffectBytesRefV1 { offset, length })
}

fn clipboard_target_to_ffi(target: ClipboardTarget) -> u32 {
    match target {
        ClipboardTarget::Clipboard => VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        ClipboardTarget::Primary => VIEM_CLIPBOARD_TARGET_PRIMARY,
    }
}

fn register_kind_to_ffi(kind: RegisterKind) -> u32 {
    match kind {
        RegisterKind::Characterwise => VIEM_REGISTER_KIND_CHARACTER,
        RegisterKind::Linewise => VIEM_REGISTER_KIND_LINE,
        RegisterKind::Blockwise => VIEM_REGISTER_KIND_BLOCK,
    }
}

fn ex_option_name_to_ffi(name: &ExOptionName) -> u32 {
    match name {
        ExOptionName::Wrap => VIEM_EX_OPTION_WRAP,
        ExOptionName::FileFormat => VIEM_EX_OPTION_FILE_FORMAT,
        ExOptionName::FileFormats => VIEM_EX_OPTION_FILE_FORMATS,
        ExOptionName::IgnoreCase => VIEM_EX_OPTION_IGNORECASE,
        ExOptionName::SmartCase => VIEM_EX_OPTION_SMARTCASE,
        ExOptionName::WrapScan => VIEM_EX_OPTION_WRAPSCAN,
        ExOptionName::HlSearch => VIEM_EX_OPTION_HLSEARCH,
        ExOptionName::IncSearch => VIEM_EX_OPTION_INCSEARCH,
        ExOptionName::TextWidth => VIEM_EX_OPTION_TEXTWIDTH,
        ExOptionName::AutoIndent => VIEM_EX_OPTION_AUTOINDENT,
        ExOptionName::TabStop => VIEM_EX_OPTION_TABSTOP,
        ExOptionName::ShiftWidth => VIEM_EX_OPTION_SHIFTWIDTH,
        ExOptionName::SoftTabStop => VIEM_EX_OPTION_SOFTTABSTOP,
        ExOptionName::ExpandTab => VIEM_EX_OPTION_EXPANDTAB,
        ExOptionName::SmartTab => VIEM_EX_OPTION_SMARTTAB,
        ExOptionName::ContinueCommentsOnEnter => VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_ENTER,
        ExOptionName::ContinueCommentsOnOpenLine => VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_OPEN_LINE,
        ExOptionName::List => VIEM_EX_OPTION_LIST,
        ExOptionName::ListChars => VIEM_EX_OPTION_LISTCHARS,
        ExOptionName::AutoSelect => VIEM_EX_OPTION_AUTOSELECT,
        ExOptionName::KeyModel => VIEM_EX_OPTION_KEYMODEL,
        ExOptionName::SelectMode => VIEM_EX_OPTION_SELECTMODE,

    }
}

fn export_ex_option(
    option: &ExOptionDisplay,
    file_formats: &mut Vec<u32>,
    strings: &mut Vec<u8>,
) -> Result<ViemExOptionDisplayV1, ViemStatus> {
    let first_file_format = checked_export_count(file_formats.len())?;
    let mut text = ViemEffectBytesRefV1::default();
    let (value_kind, scalar_value) = match &option.value {
        ExOptionValue::String(value) => { text = push_effect_text(strings, value)?; (VIEM_EX_OPTION_VALUE_STRING, 0) }
        ExOptionValue::Boolean(value) => (VIEM_EX_OPTION_VALUE_BOOLEAN, u32::from(*value)),
        ExOptionValue::FileFormat(value) => {
            (VIEM_EX_OPTION_VALUE_FILE_FORMAT, file_format_to_ffi(*value))
        }
        ExOptionValue::FileFormats(values) => {
            file_formats.extend(values.iter().copied().map(file_format_to_ffi));
            (VIEM_EX_OPTION_VALUE_FILE_FORMATS, 0)
        }
        ExOptionValue::Number(value) => (VIEM_EX_OPTION_VALUE_NUMBER, *value),
        ExOptionValue::Integer(value) => (VIEM_EX_OPTION_VALUE_NUMBER, *value as u32),
        ExOptionValue::Indentation(_) | ExOptionValue::VisibleWhitespace(_) => return Err(ViemStatus::InvalidArgument),
        ExOptionValue::OptionalNumber(value) => {
            (VIEM_EX_OPTION_VALUE_NUMBER, value.unwrap_or(0))
        }
    };
    Ok(ViemExOptionDisplayV1 {
        struct_size: VIEM_EX_OPTION_DISPLAY_V1_SIZE,
        name: ex_option_name_to_ffi(&option.name),
        value_kind,
        scalar_value,
        first_file_format,
        text,
        file_format_count: checked_export_count(file_formats.len())?
            .checked_sub(first_file_format)
            .ok_or(ViemStatus::LengthOverflow)?,
    })
}

fn set_ex_path(
    output: &mut ViemExFrontendRequestV1,
    strings: &mut Vec<u8>,
    path: Option<&str>,
) -> Result<(), ViemStatus> {
    if let Some(path) = path {
        output.flags |= VIEM_EX_FRONTEND_HAS_PATH;
        output.text = push_effect_text(strings, path)?;
    }
    Ok(())
}

fn set_ex_range(output: &mut ViemExFrontendRequestV1, range: Option<HardLineRange>) {
    if let Some(range) = range {
        output.flags |= VIEM_EX_FRONTEND_HAS_RANGE;
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
    options: &mut Vec<ViemExOptionDisplayV1>,
    marks: &mut Vec<ViemExMarkV1>,
    registers: &mut Vec<ViemExRegisterV1>,
    jumps: &mut Vec<ViemExJumpV1>,
    text_lines: &mut Vec<ViemExTextLineV1>,
    file_formats: &mut Vec<u32>,
    hard_breaks: &mut Vec<u64>,
) -> Result<ViemExFrontendRequestV1, ViemStatus> {
    let mut output = ViemExFrontendRequestV1 {
        struct_size: VIEM_EX_FRONTEND_REQUEST_V1_SIZE,
        document_id: document_id.0,
        document_revision: document_revision.0,
        ..ViemExFrontendRequestV1::default()
    };
    match request {
        ExFrontendRequest::Window(request) => {
            use crate::command::window::WindowRequest;
            output.kind = VIEM_EX_FRONTEND_WINDOW;
            let mut count = |value: Option<usize>| -> Result<(), ViemStatus> {
                if let Some(value) = value {
                    output.flags |= VIEM_EX_FRONTEND_HAS_COUNT;
                    output.window_count = checked_export_count(value)?;
                }
                Ok(())
            };
            output.window_command = match request {
                WindowRequest::FocusDown { count: rows } => {
                    count(Some(*rows))?;
                    VIEM_WINDOW_FOCUS_DOWN
                }
                WindowRequest::FocusUp { count: rows } => {
                    count(Some(*rows))?;
                    VIEM_WINDOW_FOCUS_UP
                }
                WindowRequest::FocusNext { index } => {
                    count(*index)?;
                    VIEM_WINDOW_FOCUS_NEXT
                }
                WindowRequest::FocusPrevious { index } => {
                    count(*index)?;
                    VIEM_WINDOW_FOCUS_PREVIOUS
                }
                WindowRequest::FocusTop => VIEM_WINDOW_FOCUS_TOP,
                WindowRequest::FocusBottom => VIEM_WINDOW_FOCUS_BOTTOM,
                WindowRequest::FocusLastAccessed => VIEM_WINDOW_FOCUS_LAST_ACCESSED,
                WindowRequest::RotateDown { count: steps } => {
                    count(Some(*steps))?;
                    VIEM_WINDOW_ROTATE_DOWN
                }
                WindowRequest::RotateUp { count: steps } => {
                    count(Some(*steps))?;
                    VIEM_WINDOW_ROTATE_UP
                }
                WindowRequest::Exchange { index } => {
                    count(*index)?;
                    VIEM_WINDOW_EXCHANGE
                }
                WindowRequest::MoveToTop => VIEM_WINDOW_MOVE_TO_TOP,
                WindowRequest::MoveToBottom => VIEM_WINDOW_MOVE_TO_BOTTOM,
                WindowRequest::CloseOthers => VIEM_WINDOW_CLOSE_OTHERS,
                WindowRequest::Grow { rows } => {
                    count(Some(*rows))?;
                    VIEM_WINDOW_GROW
                }
                WindowRequest::Shrink { rows } => {
                    count(Some(*rows))?;
                    VIEM_WINDOW_SHRINK
                }
                WindowRequest::SetHeight { rows } => {
                    count(*rows)?;
                    VIEM_WINDOW_SET_HEIGHT
                }
                WindowRequest::EqualizeHeights => VIEM_WINDOW_EQUALIZE_HEIGHTS,
            };
        }
        ExFrontendRequest::File(request) => match request {
            ExFileRequest::Only { force } => {
                output.kind = VIEM_EX_FRONTEND_ONLY;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::Read { path, after } => {
                output.kind = VIEM_EX_FRONTEND_READ;
                output.hard_line_start = checked_export_count(*after)?;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::Source { path } => {
                output.kind = VIEM_EX_FRONTEND_SOURCE;
                set_ex_path(&mut output, strings, Some(path))?;
            }
            ExFileRequest::File { path, truncate } => {
                output.kind = VIEM_EX_FRONTEND_FILE;
                output.flags |= u32::from(*truncate) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::NavigateArgument { target, force, write_first, path, line } => {
                use crate::command::argument_list::ExArgumentTarget;
                output.kind = VIEM_EX_FRONTEND_ARGUMENT;
                (output.argument_command, output.argument_count) = match target {
                    ExArgumentTarget::Next(count) => (VIEM_ARGUMENT_NEXT, *count),
                    ExArgumentTarget::Previous(count) => (VIEM_ARGUMENT_PREVIOUS, *count),
                    ExArgumentTarget::First => (VIEM_ARGUMENT_FIRST, 1),
                    ExArgumentTarget::Last => (VIEM_ARGUMENT_LAST, 1),
                    ExArgumentTarget::Index(index) => (VIEM_ARGUMENT_INDEX, *index),
                    ExArgumentTarget::Current => (VIEM_ARGUMENT_CURRENT, 1),
                };
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                output.flags |= u32::from(*write_first) * VIEM_EX_FRONTEND_WRITE_FIRST;
                if let Some(line) = line {
                    output.flags |= VIEM_EX_FRONTEND_HAS_LINE;
                    output.argument_line = *line;
                }
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::EditNewWindow { path } => {
                output.kind = VIEM_EX_FRONTEND_EDIT_NEW_WINDOW;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::CheckTime => {
                output.kind = VIEM_EX_FRONTEND_CHECKTIME;
            }
            ExFileRequest::PrintWorkingDirectory => {
                output.kind = VIEM_EX_FRONTEND_PWD;
            }
            ExFileRequest::ChangeDirectory { path } => {
                output.kind = VIEM_EX_FRONTEND_CD;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::Split { path, height } => {
                output.kind = VIEM_EX_FRONTEND_SPLIT;
                set_ex_path(&mut output, strings, path.as_deref())?;
                if let Some(height) = height {
                    output.flags |= VIEM_EX_FRONTEND_HAS_COUNT;
                    output.window_count = checked_export_count(*height)?;
                }
            }
            ExFileRequest::NewPane { height } => {
                output.kind = VIEM_EX_FRONTEND_NEW_PANE;
                if let Some(height) = height {
                    output.flags |= VIEM_EX_FRONTEND_HAS_COUNT;
                    output.window_count = checked_export_count(*height)?;
                }
            }
            ExFileRequest::Edit { path, force } => {
                output.kind = VIEM_EX_FRONTEND_EDIT;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::New { force } => {
                output.kind = VIEM_EX_FRONTEND_NEW;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::Write { path, force, range } => {
                output.kind = VIEM_EX_FRONTEND_WRITE;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
                set_ex_range(&mut output, *range);
            }
            ExFileRequest::SaveAs { path, force } => {
                output.kind = VIEM_EX_FRONTEND_SAVE_AS;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, Some(path))?;
            }
            ExFileRequest::Quit { force } => {
                output.kind = VIEM_EX_FRONTEND_QUIT;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::QuitAll { force } => {
                output.kind = VIEM_EX_FRONTEND_QUIT_ALL;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::WriteQuit { path, force, range } => {
                output.kind = VIEM_EX_FRONTEND_WRITE_QUIT;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
                set_ex_range(&mut output, *range);
            }
            ExFileRequest::Xit { path, force } => {
                output.kind = VIEM_EX_FRONTEND_XIT;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::WriteAll { force } => {
                output.kind = VIEM_EX_FRONTEND_WRITE_ALL;
                output.flags |= u32::from(*force) * VIEM_EX_FRONTEND_FORCE;
            }
        },
        ExFrontendRequest::Info(request) => match request {
            ExInfoRequest::Message(message) => {
                output.kind = VIEM_EX_FRONTEND_MESSAGE;
                output.text = push_effect_text(strings, message)?;
            }
            ExInfoRequest::Marks(names) => {
                output.kind = VIEM_EX_FRONTEND_MARKS;
                let names: String = names.iter().collect();
                output.text = push_effect_text(strings, &names)?;
                let Some(OwnedExInfoPayload::Marks(resolved)) = payload else {
                    return Err(ViemStatus::InternalError);
                };
                output.first_payload = checked_export_count(marks.len())?;
                for mark in resolved {
                    marks.push(ViemExMarkV1 {
                        struct_size: VIEM_EX_MARK_V1_SIZE,
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
                    .ok_or(ViemStatus::LengthOverflow)?;
            }
            ExInfoRequest::Registers(names) => {
                output.kind = VIEM_EX_FRONTEND_REGISTERS;
                let names: String = names.iter().collect();
                output.text = push_effect_text(strings, &names)?;
                let Some(OwnedExInfoPayload::Registers(resolved)) = payload else {
                    return Err(ViemStatus::InternalError);
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
                    registers.push(ViemExRegisterV1 {
                        struct_size: VIEM_EX_REGISTER_V1_SIZE,
                        name: u32::from(register.name),
                        register_kind: register_kind_to_ffi(register.value.kind),
                        reserved: 0,
                        text: push_effect_text(strings, &register.value.text)?,
                        first_hard_break,
                        hard_break_count: checked_export_count(hard_breaks.len())?
                            .checked_sub(first_hard_break)
                            .ok_or(ViemStatus::LengthOverflow)?,
                    });
                }
                output.payload_count = checked_export_count(registers.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(ViemStatus::LengthOverflow)?;
            }
            ExInfoRequest::Jumps => {
                output.kind = VIEM_EX_FRONTEND_JUMPS;
                let Some(OwnedExInfoPayload::Jumps(resolved)) = payload else {
                    return Err(ViemStatus::InternalError);
                };
                output.first_payload = checked_export_count(jumps.len())?;
                for jump in resolved {
                    jumps.push(ViemExJumpV1 {
                        struct_size: VIEM_EX_JUMP_V1_SIZE,
                        flags: u32::from(jump.current) * VIEM_EX_JUMP_CURRENT,
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
                    .ok_or(ViemStatus::LengthOverflow)?;
            }
            ExInfoRequest::Options(displays) => {
                output.kind = VIEM_EX_FRONTEND_OPTIONS;
                output.first_option = checked_export_count(options.len())?;
                for display in displays {
                    options.push(export_ex_option(display, file_formats, strings)?);
                }
                output.option_count = checked_export_count(options.len())?
                    .checked_sub(output.first_option)
                    .ok_or(ViemStatus::LengthOverflow)?;
            }
            ExInfoRequest::PrintLines {
                range,
                number,
                list,
            } => {
                output.kind = VIEM_EX_FRONTEND_PRINT_LINES;
                output.flags |= u32::from(*number) * VIEM_EX_FRONTEND_NUMBER;
                output.flags |= u32::from(*list) * VIEM_EX_FRONTEND_LIST;
                set_ex_range(&mut output, Some(*range));
                let Some(OwnedExInfoPayload::TextLines(resolved)) = payload else {
                    return Err(ViemStatus::InternalError);
                };
                output.first_payload = checked_export_count(text_lines.len())?;
                for line in resolved {
                    text_lines.push(ViemExTextLineV1 {
                        struct_size: VIEM_EX_TEXT_LINE_V1_SIZE,
                        reserved: 0,
                        hard_line_index: checked_export_count(line.hard_line_index)?,
                        utf8_start: checked_export_count(line.utf8_range.start)?,
                        utf8_end: checked_export_count(line.utf8_range.end)?,
                        text: push_effect_text(strings, &line.text)?,
                    });
                }
                output.payload_count = checked_export_count(text_lines.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(ViemStatus::LengthOverflow)?;
            }
        },
        ExFrontendRequest::Normal(request) => {
            output.kind = VIEM_EX_FRONTEND_NORMAL;
            output.flags |= u32::from(request.literal) * VIEM_EX_FRONTEND_LITERAL;
            output.text = push_effect_text(strings, &request.commands)?;
            set_ex_range(&mut output, Some(request.range));
        }
    }
    Ok(output)
}

fn export_effect_batch(
    handle: ViemEffectBatchHandle,
    batch: &OwnedEffectBatch,
) -> Result<EffectBatchExport, ViemStatus> {
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
        let mut register_kind = VIEM_REGISTER_KIND_NONE;
        if let Some(portable) = request.content().portable_register() {
            write_flags |= VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER;
            register_kind = register_kind_to_ffi(portable.kind);
            hard_breaks.extend(
                portable
                    .hard_break_offsets()
                    .iter()
                    .map(|offset| *offset as u64),
            );
        }
        clipboard_writes.push(ViemClipboardWriteV1 {
            struct_size: VIEM_CLIPBOARD_WRITE_V1_SIZE,
            flags: write_flags,
            target: clipboard_target_to_ffi(request.target()),
            register_kind,
            document_id: batch.document_id.0,
            document_revision: batch.document_revision.0,
            plain_text: push_effect_text(&mut strings, request.content().plain_text())?,
            first_hard_break,
            hard_break_count: checked_export_count(hard_breaks.len())?
                .checked_sub(first_hard_break)
                .ok_or(ViemStatus::LengthOverflow)?,
        });
    }

    if let Some(ex) = &batch.ex_outcome {
        flags |= VIEM_EFFECT_BATCH_HAS_EX_OUTCOME;
        if ex.document_changed {
            flags |= VIEM_EFFECT_BATCH_EX_DOCUMENT_CHANGED;
        }
        if let Some(navigation) = ex.navigation {
            flags |= VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION;
            match navigation {
                ExNavigation::TextOffset(offset) => navigation_utf8_offset = offset as u64,
                ExNavigation::HistoryRestoration => {
                    flags |= VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY;
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
        info: ViemEffectBatchInfoV1 {
            struct_size: VIEM_EFFECT_BATCH_INFO_V1_SIZE,
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
    next_handle: ViemEffectBatchHandle,
    batches: HashMap<ViemEffectBatchHandle, EffectBatchRegistryEntry>,
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
    handle: ViemEffectBatchHandle,
    committed: bool,
}

impl EffectBatchReservation {
    fn commit(mut self, batch: OwnedEffectBatch) -> Result<ViemEffectBatchHandle, ViemStatus> {
        let mut registry = effect_batch_registry()
            .lock()
            .map_err(|_| ViemStatus::InternalError)?;
        let entry = registry
            .batches
            .get_mut(&self.handle)
            .ok_or(ViemStatus::InternalError)?;
        if !matches!(entry, EffectBatchRegistryEntry::Reserved) {
            return Err(ViemStatus::InternalError);
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

fn reserve_effect_batch() -> Result<EffectBatchReservation, ViemStatus> {
    let mut registry = effect_batch_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    let handle = registry.next_handle;
    if handle == 0 || registry.batches.contains_key(&handle) {
        return Err(ViemStatus::ResourceExhausted);
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

fn owned_effect_batch(handle: ViemEffectBatchHandle) -> Result<Arc<OwnedEffectBatch>, ViemStatus> {
    if handle == 0 {
        return Err(ViemStatus::InvalidHandle);
    }
    let registry = effect_batch_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    match registry.batches.get(&handle) {
        Some(EffectBatchRegistryEntry::Ready(batch)) => Ok(batch.clone()),
        Some(EffectBatchRegistryEntry::Reserved) | None => Err(ViemStatus::InvalidHandle),
    }
}

enum CoreRegistryEntry {
    Ready(Core<CTextMeasurementProvider>),
    Busy,
}

struct CoreRegistry {
    next_handle: ViemCoreHandle,
    cores: HashMap<ViemCoreHandle, CoreRegistryEntry>,
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
    handle: ViemCoreHandle,
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

fn register_core(core: Core<CTextMeasurementProvider>) -> Result<ViemCoreHandle, ViemStatus> {
    let mut registry = core_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    let handle = registry.next_handle;
    if handle == 0 {
        return Err(ViemStatus::ResourceExhausted);
    }
    if registry.cores.contains_key(&handle) {
        return Err(ViemStatus::InternalError);
    }
    registry.next_handle = handle.checked_add(1).unwrap_or(0);
    registry
        .cores
        .insert(handle, CoreRegistryEntry::Ready(core));
    Ok(handle)
}

fn checkout_core(handle: ViemCoreHandle) -> Result<CoreLease, ViemStatus> {
    if handle == 0 {
        return Err(ViemStatus::InvalidHandle);
    }
    let mut registry = core_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    let entry = registry
        .cores
        .get_mut(&handle)
        .ok_or(ViemStatus::InvalidHandle)?;
    let CoreRegistryEntry::Ready(_) = entry else {
        return Err(ViemStatus::CoreBusy);
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
    handle: ViemCoreHandle,
    operation: impl FnOnce(&Core<CTextMeasurementProvider>) -> Result<R, ViemStatus>,
) -> Result<R, ViemStatus> {
    let core = checkout_core(handle)?;
    operation(core.core())
}

fn with_core_mut<R>(
    handle: ViemCoreHandle,
    operation: impl FnOnce(&mut Core<CTextMeasurementProvider>) -> Result<R, ViemStatus>,
) -> Result<R, ViemStatus> {
    let mut core = checkout_core(handle)?;
    operation(core.core_mut())
}

/// Return the ABI version without consulting any document state.
#[no_mangle]
pub extern "C" fn viem_core_abi_version() -> u32 {
    catch_unwind(|| VIEM_CORE_ABI_VERSION).unwrap_or(0)
}

fn create_document_from_source(
    bytes: Vec<u8>,
    options: ViemDocumentOptions,
) -> Result<Document, ViemStatus> {
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

fn parse_execution_context(raw: u32) -> Result<LayoutExecutionContext, ViemStatus> {
    match raw {
        VIEM_LAYOUT_EXECUTION_WORKER_POOL => Ok(LayoutExecutionContext::WorkerPool),
        VIEM_LAYOUT_EXECUTION_DEDICATED_SERIAL => {
            Ok(LayoutExecutionContext::DedicatedSerialExecutor)
        }
        VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN => Ok(LayoutExecutionContext::FrontendMainThread),
        _ => Err(ViemStatus::InvalidArgument),
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

fn parse_key(input: ViemKeyInputV1) -> Result<Key, ViemStatus> {
    if input.struct_size < VIEM_KEY_INPUT_V1_SIZE
        || input.modifiers & !15 != 0
        || (input.kind != VIEM_KEY_FUNCTION && input.modifiers != 0 && !matches!(input.kind, VIEM_KEY_LEFT | VIEM_KEY_RIGHT | VIEM_KEY_WORD_LEFT | VIEM_KEY_WORD_RIGHT | VIEM_KEY_UP | VIEM_KEY_DOWN | VIEM_KEY_HOME | VIEM_KEY_END | VIEM_KEY_DOCUMENT_START | VIEM_KEY_DOCUMENT_END | VIEM_KEY_PAGE_UP | VIEM_KEY_PAGE_DOWN))
    {
        return Err(ViemStatus::InvalidArgument);
    }
    let scalar = || char::from_u32(input.codepoint).ok_or(ViemStatus::InvalidKey);
    let special = |key| {
        if input.codepoint == 0 {
            if input.modifiers != 0 {
                Ok(Key::ModifiedNavigation { key: crate::command::NavigationKey::from_key(key).ok_or(ViemStatus::InvalidKey)?, modifiers: input.modifiers as u8 })
            } else { Ok(key) }
        } else {
            Err(ViemStatus::InvalidKey)
        }
    };
    match input.kind {
        VIEM_KEY_CHARACTER => Ok(Key::Char(scalar()?)),
        VIEM_KEY_ESCAPE => special(Key::Escape),
        VIEM_KEY_COPY_SELECTION => special(Key::CopySelection),
        VIEM_KEY_ENTER => special(Key::Enter),
        VIEM_KEY_SHIFT_ENTER => special(Key::ShiftEnter),
        VIEM_KEY_TAB => special(Key::Tab),
        VIEM_KEY_BACK_TAB => special(Key::BackTab),
        VIEM_KEY_BACKSPACE => special(Key::Backspace),
        VIEM_KEY_DELETE => special(Key::Delete),
        VIEM_KEY_LEFT => special(Key::Left),
        VIEM_KEY_RIGHT => special(Key::Right),
        VIEM_KEY_WORD_LEFT => special(Key::WordLeft),
        VIEM_KEY_WORD_RIGHT => special(Key::WordRight),
        VIEM_KEY_UP => special(Key::Up),
        VIEM_KEY_DOWN => special(Key::Down),
        VIEM_KEY_HOME => special(Key::Home),
        VIEM_KEY_END => special(Key::End),
        VIEM_KEY_DOCUMENT_START => special(Key::DocumentStart),
        VIEM_KEY_DOCUMENT_END => special(Key::DocumentEnd),
        VIEM_KEY_PAGE_UP => special(Key::PageUp),
        VIEM_KEY_PAGE_DOWN => special(Key::PageDown),
        VIEM_KEY_CONTROL_CHARACTER => Ok(Key::Ctrl(scalar()?)),
        VIEM_KEY_FUNCTION if (1..=35).contains(&input.codepoint) => Ok(Key::Function {
            number: input.codepoint as u8,
            modifiers: input.modifiers as u8,
        }),
        _ => Err(ViemStatus::InvalidKey),
    }
}

fn mode_to_ffi(mode: Mode, select: bool, native: bool) -> u32 {
    if native { return match mode { Mode::VisualCharacter => VIEM_MODE_SELECTION_CHARACTER, Mode::VisualLine => VIEM_MODE_SELECTION_LINE, Mode::VisualBlock => VIEM_MODE_SELECTION_BLOCK, _ => unreachable!() }; }
    if select { return match mode { Mode::VisualCharacter => VIEM_MODE_SELECT_CHARACTER, Mode::VisualLine => VIEM_MODE_SELECT_LINE, Mode::VisualBlock => VIEM_MODE_SELECT_BLOCK, _ => unreachable!() }; }
    match mode {
        Mode::Normal => VIEM_MODE_NORMAL,
        Mode::Insert => VIEM_MODE_INSERT,
        Mode::Replace => VIEM_MODE_REPLACE,
        Mode::VisualCharacter => VIEM_MODE_VISUAL_CHARACTER,
        Mode::VisualLine => VIEM_MODE_VISUAL_LINE,
        Mode::VisualBlock => VIEM_MODE_VISUAL_BLOCK,
        Mode::CommandLine => VIEM_MODE_COMMAND_LINE,
    }
}

fn command_status_to_ffi(status: &CommandStatus) -> u32 {
    match status {
        CommandStatus::Complete => VIEM_COMMAND_STATUS_COMPLETE,
        CommandStatus::Pending => VIEM_COMMAND_STATUS_PENDING,
        CommandStatus::Cancelled => VIEM_COMMAND_STATUS_CANCELLED,
        CommandStatus::NeedsMoreLayout(_) => VIEM_COMMAND_STATUS_NEEDS_MORE_LAYOUT,
        CommandStatus::SearchNotFound => VIEM_COMMAND_STATUS_SEARCH_NOT_FOUND,
        CommandStatus::Unsupported(_) => VIEM_COMMAND_STATUS_UNSUPPORTED,
        CommandStatus::ExError(crate::command::ExCommandError::Execute(
            crate::command::ex_execute::ExExecuteError::ReadOnly,
        )) => VIEM_COMMAND_STATUS_READ_ONLY,
        CommandStatus::Error(_)
        | CommandStatus::CountError(_)
        | CommandStatus::RegisterReadError(_)
        | CommandStatus::RegisterWriteError(_)
        | CommandStatus::ExError(_)
        | CommandStatus::VisualBlockError(_) => VIEM_COMMAND_STATUS_ERROR,
    }
}

fn layout_status(error: LayoutError) -> ViemStatus {
    match error {
        LayoutError::Measurement(MeasurementError::UnstableShapingContext(_)) => {
            ViemStatus::UnstableShapingContext
        }
        LayoutError::Measurement(_)
        | LayoutError::StaleMeasurementResponse
        | LayoutError::MeasurementEnvironmentChangedDuringShape
        | LayoutError::MetricsChangedDuringShape
        | LayoutError::MalformedMeasurement(_) => ViemStatus::ProviderFailure,
        LayoutError::InvalidScale
        | LayoutError::InvalidStyle
        | LayoutError::InvalidStyleRun { .. }
        | LayoutError::InvalidGeometry => ViemStatus::InvalidArgument,
        _ => ViemStatus::CoreFailure,
    }
}

fn style_error_status(error: StyleError) -> ViemStatus {
    match error {
        StyleError::UnknownStyle(_) => ViemStatus::UnknownStyle,
        StyleError::DefinitionNotGeneratedConfiguration { .. } => ViemStatus::StyleReadOnly,
        StyleError::InheritanceCycle(_) => ViemStatus::StyleInheritanceCycle,
        StyleError::IncompatibleBlockRole { .. }
        | StyleError::InapplicableBlockProperties { .. }
        | StyleError::InapplicableStyleProperty { .. } => ViemStatus::IncompatibleStyleRole,
        StyleError::MissingParent(_)
        | StyleError::InapplicableNextParagraphStyle { .. }
        | StyleError::InvalidNextParagraphStyle { .. }
        | StyleError::InapplicableStyleRelationship(_)
        | StyleError::CannotReplaceBaseStyle(_)
        | StyleError::CannotRemoveBaseStyle(_)
        | StyleError::InvalidBaseStyleDefinition(_)
        | StyleError::StyleInUse(_) => ViemStatus::InvalidStyleRelationship,
        StyleError::InvalidCharacterProperties(_)
        | StyleError::InvalidBlockProperties(_)
        | StyleError::InvalidStylePropertyValue { .. }
        | StyleError::InvalidDefinitionMetadata(_) => ViemStatus::InvalidStyleValue,
        StyleError::StyleSheetRevisionExhausted => ViemStatus::ResourceExhausted,
        StyleError::StyleAlreadyExists(_) => ViemStatus::InvalidArgument,
    }
}

fn style_transaction_status(error: StyleTransactionError) -> ViemStatus {
    match error {
        StyleTransactionError::Definition(error) => style_error_status(error),
        StyleTransactionError::Unsupported { .. }
        | StyleTransactionError::TranslationUnavailable => ViemStatus::UnsupportedOperation,
        StyleTransactionError::NeedsPolicy(_) => ViemStatus::PolicyRequired,
        StyleTransactionError::DefinitionReadOnly(_) => ViemStatus::StyleReadOnly,
        StyleTransactionError::ConfigurationIntentRequired(_)
        | StyleTransactionError::InvalidPropertyTarget { .. } => ViemStatus::InvalidStyleValue,
    }
}

fn model_transaction_status(error: ModelTransactionError) -> ViemStatus {
    match error {
        ModelTransactionError::Document(error) => document_status(error),
        ModelTransactionError::Style(error) => style_transaction_status(error),
        ModelTransactionError::WrongDocument { .. } => ViemStatus::InvalidArgument,
        ModelTransactionError::StaleRevision { .. } | ModelTransactionError::StaleDocumentState => {
            ViemStatus::StaleRevision
        }
        ModelTransactionError::RevisionExhausted => ViemStatus::ResourceExhausted,
        ModelTransactionError::Position(_) | ModelTransactionError::History(_) => {
            ViemStatus::CoreFailure
        }
    }
}

fn core_status(error: CoreError) -> ViemStatus {
    match error {
        CoreError::UnknownView(_) => ViemStatus::InvalidView,
        CoreError::IdentifierExhausted(_) => ViemStatus::ResourceExhausted,
        CoreError::Document(error) => document_status(error),
        CoreError::ModelTransaction(error) => model_transaction_status(error),
        CoreError::StaleStyleSheet { .. } => ViemStatus::StaleRevision,
        CoreError::StyleEditGroup(error) => match error {
            StyleEditGroupError::AlreadyActive(_) => ViemStatus::StyleEditGroupActive,
            StyleEditGroupError::WrongOwner { .. } => ViemStatus::StyleEditGroupWrongOwner,
            StyleEditGroupError::WrongDocument { .. } => ViemStatus::InvalidArgument,
            StyleEditGroupError::NoActiveGroup
            | StyleEditGroupError::WrongGroup { .. }
            | StyleEditGroupError::IdentityMismatch => ViemStatus::InvalidStyleEditGroup,
        },
        CoreError::Composition(error) => composition_status(error),
        CoreError::Completion(error) => match error {
            crate::command::completion::CompletionError::Composition(error) => {
                composition_status(error)
            }
            crate::command::completion::CompletionError::Document(error) => document_status(error),
            crate::command::completion::CompletionError::IdentityExhausted => {
                ViemStatus::ResourceExhausted
            }
            crate::command::completion::CompletionError::Search(error) => match error {
                crate::document::PositionError::WrongSnapshot { .. } => ViemStatus::StaleRevision,
                crate::document::PositionError::WrongDocument { .. }
                | crate::document::PositionError::WrongDomain { .. }
                | crate::document::PositionError::WrongSourcePart { .. } => ViemStatus::InvalidArgument,
                crate::document::PositionError::InvalidBoundary { .. }
                | crate::document::PositionError::InvertedRange { .. } => ViemStatus::InvalidRange,
                crate::document::PositionError::InvalidUnicodeBoundary { .. } => {
                    ViemStatus::NotGraphemeBoundary
                }
                crate::document::PositionError::ArithmeticOverflow => ViemStatus::LengthOverflow,
                _ => ViemStatus::CoreFailure,
            },
        },
        CoreError::Layout(error) => layout_status(error),
        CoreError::LayoutJob(LayoutJobError::Layout(error)) => layout_status(error),
        CoreError::LayoutJob(
            LayoutJobError::ProviderThreadingChanged { .. }
            | LayoutJobError::WrongMeasurementEnvironment { .. }
            | LayoutJobError::StaleMetrics { .. },
        ) => ViemStatus::LayoutUnavailable,
        CoreError::Persistence(crate::document::PersistenceError::ReadOnly) => {
            ViemStatus::PolicyRequired
        }
        CoreError::NoVisualSelection => ViemStatus::InvalidRange,
        CoreError::StaleLogicalSelection => ViemStatus::StaleRevision,
        _ => ViemStatus::CoreFailure,
    }
}

fn composition_status(error: CompositionError) -> ViemStatus {
    match error {
        CompositionError::AlreadyActive
        | CompositionError::NoActiveSession
        | CompositionError::WrongDocument { .. } => ViemStatus::InvalidArgument,
        CompositionError::StaleRevision { .. } => ViemStatus::StaleRevision,
        CompositionError::InvalidRange { .. } => ViemStatus::InvalidRange,
        CompositionError::InvalidGraphemeBoundary { .. } => ViemStatus::NotGraphemeBoundary,
        CompositionError::Document(error) => document_status(error),
        CompositionError::UnresolvableCommitCaret
        | CompositionError::Position(_)
        | CompositionError::Transaction(_) => ViemStatus::CoreFailure,
    }
}

fn summarize_core_outcome(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
    outcome: Option<&CoreOutcome>,
) -> Result<ViemCoreOutcomeV1, ViemStatus> {
    let command_state = core.command_state(view_id).ok_or(ViemStatus::InvalidView)?;
    let layout = core
        .presentation_layout(view_id)
        .ok_or(ViemStatus::InvalidView)?;
    let mut flags = 0;
    let command_status = if let Some(command) = outcome.and_then(|value| value.command.as_ref()) {
        flags |= VIEM_OUTCOME_HAS_COMMAND;
        if command.cursor_moved {
            flags |= VIEM_OUTCOME_CURSOR_MOVED;
        }
        if command.mode_changed {
            flags |= VIEM_OUTCOME_MODE_CHANGED;
        }
        if command.ex_outcome.is_some() || !command.clipboard_writes.is_empty() {
            flags |= VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS;
        }
        command_status_to_ffi(&command.status)
    } else {
        VIEM_COMMAND_STATUS_NONE
    };
    if outcome.is_some_and(|value| value.document_changed) {
        flags |= VIEM_OUTCOME_DOCUMENT_CHANGED;
    }
    if outcome.is_some_and(|value| value.layout_changed) {
        flags |= VIEM_OUTCOME_LAYOUT_CHANGED;
    }
    if outcome.is_some_and(|value| value.position_map.is_some()) {
        flags |= VIEM_OUTCOME_HAS_POSITION_MAP;
    }
    if outcome.is_some_and(|value| !value.composition_changes.is_empty()) {
        flags |= VIEM_OUTCOME_HAS_COMPOSITION_CHANGES;
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
            flags |= VIEM_OUTCOME_HAS_LAYOUT;
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
    Ok(ViemCoreOutcomeV1 {
        struct_size: VIEM_CORE_OUTCOME_V1_SIZE,
        command_status,
        mode: mode_to_ffi(command_state.mode(), command_state.is_select_mode(), command_state.is_native_selection()),
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
) -> Result<ViemViewportStateV1, ViemStatus> {
    let state = core.viewport_state(view_id).map_err(core_status)?;
    let mut flags = 0;
    if state.wrap() {
        flags |= VIEM_VIEWPORT_STATE_WRAP;
    }
    // Reserved compatibility flag: wrapping always uses Unicode word boundaries.
    flags |= VIEM_VIEWPORT_STATE_LINEBREAK;
    let maximum_left = if let Some(maximum_left) = state.maximum_left() {
        flags |= VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT;
        maximum_left
    } else {
        state.estimated_maximum_left()
    };
    let maximum_top = if let Some(maximum_top) = state.maximum_top() {
        flags |= VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT;
        maximum_top
    } else {
        state.estimated_maximum_top()
    };
    if state.top_is_exact() {
        flags |= VIEM_VIEWPORT_STATE_TOP_EXACT;
    }
    let layout_revision = if let Some(layout_revision) = state.layout_revision() {
        flags |= VIEM_VIEWPORT_STATE_HAS_LAYOUT;
        layout_revision.0
    } else {
        0
    };
    Ok(ViemViewportStateV1 {
        struct_size: VIEM_VIEWPORT_STATE_V1_SIZE,
        flags,
        left: state.left(),
        top: state.top(),
        maximum_left,
        maximum_top,
        scale: state.scale(),
        document_id: state.document_id().0,
        document_revision: state.document_revision().0,
        layout_revision,
        configuration_generation: state.configuration_generation().0,
        measurement_environment_id: state.measurement_environment_id().0,
        metrics_generation: state.metrics_generation().0,
    })
}

fn summarize_document_state(document: &Document) -> ViemDocumentStateV1 {
    let history = document.history_status();
    let mut flags = 0;
    if document.has_bom() {
        flags |= VIEM_DOCUMENT_STATE_HAS_BOM;
    }
    if history.can_undo {
        flags |= VIEM_DOCUMENT_STATE_CAN_UNDO;
    }
    if history.can_redo {
        flags |= VIEM_DOCUMENT_STATE_CAN_REDO;
    }
    if history.is_dirty {
        flags |= VIEM_DOCUMENT_STATE_IS_DIRTY;
    }
    if document.is_read_only() {
        flags |= VIEM_DOCUMENT_STATE_READ_ONLY;
    }
    if document.is_recovered() {
        flags |= VIEM_DOCUMENT_STATE_RECOVERED;
    }

    ViemDocumentStateV1 {
        struct_size: VIEM_DOCUMENT_STATE_V1_SIZE,
        flags,
        document_id: document.id().0,
        document_revision: document.revision().0,
        style_sheet_revision: document.projection().style_sheet().revision.0,
        source_byte_count: document.source_byte_len() as u64,
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
) -> Result<&LayoutSnapshot, ViemStatus> {
    let layout = core
        .presentation_layout(view_id)
        .ok_or(ViemStatus::InvalidView)?;
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
        .ok_or(ViemStatus::LayoutUnavailable)
}

fn composition_overlay_identity(
    overlay: &crate::command::composition::CompositionOverlay,
    view_id: ViewId,
) -> ViemCompositionOverlayIdentityV1 {
    ViemCompositionOverlayIdentityV1 {
        struct_size: VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE,
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
) -> Result<ViemCompositionOverlayInfoV1, ViemStatus> {
    let replacement = overlay.replacement_range();
    let marked = overlay.marked_range();
    let selected = overlay.selected_range_in_overlay();
    Ok(ViemCompositionOverlayInfoV1 {
        struct_size: VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE,
        flags: VIEM_COMPOSITION_OVERLAY_ACTIVE,
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
    expected: ViemCompositionOverlayIdentityV1,
    overlay: &crate::command::composition::CompositionOverlay,
    view_id: ViewId,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    let actual = composition_overlay_identity(overlay, view_id);
    (expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.generation == actual.generation)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn snapshot_identity(snapshot: &LayoutSnapshot, view_id: ViewId) -> ViemLayoutSnapshotIdentityV1 {
    ViemLayoutSnapshotIdentityV1 {
        struct_size: VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE,
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

fn checked_export_count(value: usize) -> Result<u64, ViemStatus> {
    u64::try_from(value).map_err(|_| ViemStatus::LengthOverflow)
}

fn layout_snapshot_info(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<ViemLayoutSnapshotInfoV1, ViemStatus> {
    let mut flags = 0;
    if snapshot.coverage.is_full_document() {
        flags |= VIEM_LAYOUT_SNAPSHOT_FULL_DOCUMENT;
    }
    if snapshot.coverage.prefix_is_exact() {
        flags |= VIEM_LAYOUT_SNAPSHOT_PREFIX_EXACT;
    }
    if snapshot.content_width_is_exact {
        flags |= VIEM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT;
    }
    if snapshot.total_height_is_exact {
        flags |= VIEM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT;
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
    Ok(ViemLayoutSnapshotInfoV1 {
        struct_size: VIEM_LAYOUT_SNAPSHOT_INFO_V1_SIZE,
        flags,
        identity: snapshot_identity(snapshot, view_id),
        viewport_width: snapshot.viewport_width,
        viewport_height: snapshot.viewport_height,
        usable_width: snapshot.usable_width,
        content_width: snapshot.content_width,
        total_height: snapshot.total_height,
        content_insets: ViemLayoutInsetsV1 {
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
        cluster_count: checked_export_count(cluster_count.ok_or(ViemStatus::LengthOverflow)?)?,
        caret_count: checked_export_count(caret_count.ok_or(ViemStatus::LengthOverflow)?)?,
    })
}

fn validate_snapshot_identity(
    expected: ViemLayoutSnapshotIdentityV1,
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
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
        .ok_or(ViemStatus::StaleRevision)
}

fn validate_viewport_origin_identity(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
    request: ViemViewportOriginV1,
) -> Result<(), ViemStatus> {
    let snapshot = current_ffi_layout_snapshot(core, view_id)?;
    (request.expected_document_id == snapshot.document_id.0
        && request.expected_document_revision == snapshot.document_revision.0
        && request.expected_layout_revision == snapshot.revision.0
        && request.expected_configuration_generation == snapshot.configuration_generation.0
        && request.expected_measurement_environment_id == snapshot.measurement_environment_id.0
        && request.expected_metrics_generation == snapshot.metrics_generation.0)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn affinity_to_ffi(affinity: BoundaryAffinity) -> u32 {
    match affinity {
        BoundaryAffinity::Upstream => VIEM_BOUNDARY_AFFINITY_UPSTREAM,
        BoundaryAffinity::Downstream => VIEM_BOUNDARY_AFFINITY_DOWNSTREAM,
    }
}

fn parse_layout_affinity(raw: u32) -> Result<BoundaryAffinity, ViemStatus> {
    match raw {
        VIEM_BOUNDARY_AFFINITY_UPSTREAM => Ok(BoundaryAffinity::Upstream),
        VIEM_BOUNDARY_AFFINITY_DOWNSTREAM => Ok(BoundaryAffinity::Downstream),
        _ => Err(ViemStatus::InvalidArgument),
    }
}

fn layout_rect_to_ffi(rect: LayoutRect) -> ViemLayoutRectV1 {
    ViemLayoutRectV1 {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn color_to_ffi(color: Color) -> ViemRgbaV1 {
    ViemRgbaV1 {
        red: color.red,
        green: color.green,
        blue: color.blue,
        alpha: color.alpha,
    }
}

fn text_paint_to_ffi(paint: &ResolvedTextPaint) -> ViemTextPaintV1 {
    let mut flags = if paint.foreground_is_default {
        VIEM_TEXT_PAINT_DEFAULT_FOREGROUND
    } else {
        0
    };
    let background = paint.background.map_or_else(ViemRgbaV1::default, |color| {
        flags |= VIEM_TEXT_PAINT_HAS_BACKGROUND;
        color_to_ffi(color)
    });
    if paint.underline {
        flags |= VIEM_TEXT_PAINT_UNDERLINE;
    }
    if paint.strikethrough {
        flags |= VIEM_TEXT_PAINT_STRIKETHROUGH;
    }
    ViemTextPaintV1 {
        struct_size: VIEM_TEXT_PAINT_V1_SIZE,
        flags,
        foreground: color_to_ffi(paint.foreground),
        background,
    }
}

fn render_threading_to_ffi(threading: RenderRunThreading) -> u32 {
    match threading {
        RenderRunThreading::AnyThread => VIEM_RENDER_THREADING_ANY,
        RenderRunThreading::DedicatedSerialExecutor => VIEM_RENDER_THREADING_DEDICATED_SERIAL,
        RenderRunThreading::FrontendMainThread => VIEM_RENDER_THREADING_FRONTEND_MAIN,
    }
}

fn render_run_to_ffi(render_run: &RenderRunHandle) -> ViemRenderRunHandleV1 {
    ViemRenderRunHandleV1 {
        owner: render_run.owner.0,
        identifier: render_run.identifier,
        metrics_generation: render_run.metrics_generation.0,
        threading: render_threading_to_ffi(render_run.threading),
        reserved: 0,
    }
}

fn caret_point_to_ffi(point: CaretPoint) -> Result<ViemLayoutCaretPointV1, ViemStatus> {
    Ok(ViemLayoutCaretPointV1 {
        struct_size: VIEM_LAYOUT_CARET_POINT_V1_SIZE,
        affinity: affinity_to_ffi(point.affinity),
        document_id: point.document_id.0,
        document_revision: point.document_revision.0,
        layout_revision: point.layout_revision.0,
        text_offset: checked_export_count(point.text_offset)?,
    })
}

fn layout_query_status(error: LayoutError) -> ViemStatus {
    match error {
        LayoutError::InvalidGeometry => ViemStatus::InvalidArgument,
        LayoutError::InvalidTextOffset(_) => ViemStatus::InvalidRange,
        LayoutError::InvalidGraphemeBoundary { .. } => ViemStatus::NotGraphemeBoundary,
        LayoutError::WrongDocument
        | LayoutError::WrongDocumentRevision
        | LayoutError::StaleLayout { .. } => ViemStatus::StaleRevision,
        LayoutError::OutsideMaterializedCoverage
        | LayoutError::NoRows
        | LayoutError::NoCaretStops
        | LayoutError::NotACaretStop { .. }
        | LayoutError::LongLineSliceNeedsMoreText { .. } => ViemStatus::OutsideLayoutCoverage,
        other => layout_status(other),
    }
}

struct LayoutSnapshotExport {
    info: ViemLayoutSnapshotInfoV1,
    rows: Vec<ViemVisualRowV1>,
    clusters: Vec<ViemPositionedClusterV1>,
    carets: Vec<ViemPositionedCaretV1>,
}

struct LayoutPaintExport {
    info: ViemLayoutPaintInfoV1,
    runs: Vec<ViemPaintStyleRunV1>,
}

struct StyleSheetExport {
    info: ViemStyleSheetInfoV1,
    definitions: Vec<ViemStyleDefinitionV1>,
    properties: Vec<ViemStylePropertyV1>,
    value_items: Vec<ViemStyleValueItemV1>,
    dependencies: Vec<ViemStyleDependencyV1>,
    strings: Vec<u8>,
}

fn style_sheet_identity(document: &Document) -> ViemStyleSheetIdentityV1 {
    ViemStyleSheetIdentityV1 {
        struct_size: VIEM_STYLE_SHEET_IDENTITY_V1_SIZE,
        reserved: 0,
        document_id: document.id().0,
        document_revision: document.revision().0,
        style_sheet_revision: document.projection().style_sheet().revision.0,
    }
}

fn code_style_identity(sheet: &crate::document::StyleSheet) -> ViemStyleSheetIdentityV1 {
    ViemStyleSheetIdentityV1 { struct_size: VIEM_STYLE_SHEET_IDENTITY_V1_SIZE, reserved:0, document_id:0, document_revision:0, style_sheet_revision:sheet.revision.0 }
}
fn validate_code_style_identity(expected: ViemStyleSheetIdentityV1, sheet: &crate::document::StyleSheet) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_STYLE_SHEET_IDENTITY_V1_SIZE || expected.reserved != 0 { return Err(ViemStatus::InvalidArgument); }
    if expected.document_id != 0 || expected.document_revision != 0 || expected.style_sheet_revision != sheet.revision.0 { return Err(ViemStatus::StaleRevision); }
    Ok(())
}
fn export_code_style_snapshot(sheet: &crate::document::StyleSheet) -> Result<StyleSheetExport, ViemStatus> {
    export_style_sheet_snapshot(sheet, code_style_identity(sheet), Format::Code)
}
fn export_code_style_sheet() -> Result<StyleSheetExport, ViemStatus> { export_code_style_snapshot(&crate::document::code_style::snapshot()) }

/// # Safety
/// Output must be one writable aligned style-info record.
#[no_mangle]
pub unsafe extern "C" fn viem_code_style_sheet_info(output: *mut ViemStyleSheetInfoV1) -> ViemStatus {
    unsafe { viem_core_style_sheet_info(0, output) }
}

/// # Safety
/// Same bounded, disjoint output-array contract as viem_core_copy_style_sheet.
#[no_mangle]
pub unsafe extern "C" fn viem_code_copy_style_sheet(expected: *const ViemStyleSheetIdentityV1, definitions: *mut ViemStyleDefinitionV1, definition_capacity:u64, properties:*mut ViemStylePropertyV1, property_capacity:u64, value_items:*mut ViemStyleValueItemV1, value_item_capacity:u64, dependencies:*mut ViemStyleDependencyV1, dependency_capacity:u64, string_bytes:*mut u8, string_capacity:u64, out_info:*mut ViemStyleSheetInfoV1) -> ViemStatus {
    unsafe { viem_core_copy_style_sheet(0,expected,definitions,definition_capacity,properties,property_capacity,value_items,value_item_capacity,dependencies,dependency_capacity,string_bytes,string_capacity,out_info) }
}

/// # Safety
/// Request and nested slices must be valid and disjoint from the output.
#[no_mangle]
pub unsafe extern "C" fn viem_code_edit_style(request:*const ViemStyleEditV1, output:*mut ViemStyleSheetInfoV1) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { parse_style_edit_request(request, output)? };
        let sheet = crate::document::code_style::snapshot();
        validate_code_style_identity(request.identity, &sheet)?;
        let edit = sheet.prepare_generated_field_edit(request.namespace,&request.style,&request.edit).map_err(style_error_status)?;
        let sheet = crate::document::code_style::edit(sheet.revision,edit).map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe { output.write(export_code_style_snapshot(&sheet)?.info); }
        Ok(())
    })
}

/// # Safety
/// Request and all nested slices must be readable and disjoint from output.
#[no_mangle]
pub unsafe extern "C" fn viem_code_create_style(request:*const ViemCreateStyleV1, output:*mut ViemStyleSheetInfoV1) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request,output)? };
        if request.struct_size < VIEM_CREATE_STYLE_V1_SIZE || request.namespace != VIEM_STYLE_NAMESPACE_CHARACTER || request.next_style_id.length != 0 { return Err(ViemStatus::InvalidArgument); }
        let id = unsafe { composition_utf8(request.style_id,output)? };
        let name = unsafe { composition_utf8(request.display_name,output)? };
        let parent = unsafe { composition_utf8(request.parent_id,output)? };
        let sheet = crate::document::code_style::snapshot();
        validate_code_style_identity(request.identity,&sheet)?;
        let edit = crate::document::StyleDefinitionEdit::InsertCharacter {
            style: crate::document::CharacterStyle {id:StyleId(id),based_on:(!parent.is_empty()).then(|| StyleId(parent)),properties:Default::default()},
            metadata:crate::document::StyleDefinitionMetadata::generated(name),
        };
        let sheet = crate::document::code_style::edit(sheet.revision,edit).map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe { output.write(export_code_style_snapshot(&sheet)?.info); }
        Ok(())
    })
}

/// # Safety
/// Request and nested slices must be readable and disjoint from output.
#[no_mangle]
pub unsafe extern "C" fn viem_code_delete_style(request:*const ViemDeleteStyleV1, output:*mut ViemStyleSheetInfoV1) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request,output)? };
        if request.struct_size < VIEM_DELETE_STYLE_V1_SIZE || request.namespace != VIEM_STYLE_NAMESPACE_CHARACTER { return Err(ViemStatus::InvalidArgument); }
        let id = unsafe { composition_utf8(request.style_id,output)? };
        let sheet = crate::document::code_style::snapshot();
        validate_code_style_identity(request.identity,&sheet)?;
        let sheet = crate::document::code_style::edit(sheet.revision,crate::document::StyleDefinitionEdit::DeleteCharacter(StyleId(id))).map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe { output.write(export_code_style_snapshot(&sheet)?.info); }
        Ok(())
    })
}

/// # Safety
/// The UTF-8 name must remain readable and must not overlap the one writable
/// style-info output record.
#[no_mangle]
pub unsafe extern "C" fn viem_code_materialize_style(
    name: *const u8,
    length: u64,
    output: *mut ViemStyleSheetInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(name, length)?,
            typed_pointer_region(output, 1)?,
        ])?;
        let name = std::str::from_utf8(unsafe { input_bytes(name, length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?;
        if name.trim().is_empty() {
            return Err(ViemStatus::InvalidArgument);
        }
        let generated = crate::document::code_style::materialize([name]);
        let sheet = generated
            .sheet
            .unwrap_or_else(crate::document::code_style::snapshot);
        if crate::document::code_style::resolve_name(&sheet, name).is_none() {
            return Err(ViemStatus::ResourceExhausted);
        }
        unsafe { output.write(export_code_style_snapshot(&sheet)?.info); }
        Ok(())
    })
}

/// # Safety
/// The input slice must remain readable during this call. Empty resets defaults.
#[no_mangle]
pub unsafe extern "C" fn viem_code_replace_style_json(input:*const u8, length:u64) -> ViemStatus {
    ffi_boundary(|| { let bytes = unsafe { input_bytes(input,length)? }; crate::document::code_style::replace_json(bytes).map_err(|_| ViemStatus::InvalidArgument)?; Ok(()) })
}

/// # Safety
/// Uses the standard disjoint two-pass byte-buffer contract.
#[no_mangle]
pub unsafe extern "C" fn viem_code_export_style_json(output:*mut u8, capacity:u64, required:*mut u64) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(output,capacity)?,typed_pointer_region(required,1)?])?;
        let bytes = crate::document::code_style::export_json().map_err(|_| ViemStatus::CoreFailure)?;
        unsafe { required.write(bytes.len() as u64); }
        if capacity < bytes.len() as u64 { return Err(ViemStatus::BufferTooSmall); }
        unsafe { copy_output(&bytes,output); }
        Ok(())
    })
}

fn validate_style_sheet_identity(
    expected: ViemStyleSheetIdentityV1,
    document: &Document,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_STYLE_SHEET_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    let actual = style_sheet_identity(document);
    (expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.style_sheet_revision == actual.style_sheet_revision)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn style_role_to_ffi(role: BlockRole) -> u32 {
    match role {
        BlockRole::Document => VIEM_STYLE_ROLE_DOCUMENT,
        BlockRole::Paragraph => VIEM_STYLE_ROLE_PARAGRAPH,
        BlockRole::Quote => VIEM_STYLE_ROLE_QUOTE,
        BlockRole::CodeBlock => VIEM_STYLE_ROLE_CODE_BLOCK,
        BlockRole::List => VIEM_STYLE_ROLE_LIST,
        BlockRole::ListItem => VIEM_STYLE_ROLE_LIST_ITEM,

    }
}

fn style_origin_to_ffi(origin: StyleDefinitionOrigin) -> u32 {
    match origin {
        StyleDefinitionOrigin::SourceBacked => VIEM_STYLE_ORIGIN_SOURCE_BACKED,
        StyleDefinitionOrigin::GeneratedConfiguration => VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION,
        StyleDefinitionOrigin::SyntheticReadOnly => VIEM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY,
    }
}

fn style_property_to_ffi(property: StyleProperty) -> u32 {
    match property {
        StyleProperty::BlockMarginRight => VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT,
        StyleProperty::BlockMarginLeft => VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT,
        StyleProperty::BlockPaddingTop => VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP,
        StyleProperty::BlockPaddingRight => VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT,
        StyleProperty::BlockPaddingBottom => VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM,
        StyleProperty::BlockPaddingLeft => VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT,
        StyleProperty::BlockBorderTopWidth => VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH,
        StyleProperty::BlockBorderTopColor => VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR,
        StyleProperty::BlockBorderRightWidth => VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH,
        StyleProperty::BlockBorderRightColor => VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR,
        StyleProperty::BlockBorderBottomWidth => VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH,
        StyleProperty::BlockBorderBottomColor => VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR,
        StyleProperty::BlockBorderLeftWidth => VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH,
        StyleProperty::BlockBorderLeftColor => VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR,
        StyleProperty::BlockBackground => VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND,
        StyleProperty::CanvasBackground => VIEM_STYLE_PROPERTY_CANVAS_BACKGROUND,
        StyleProperty::CanvasPaddingTop => VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
        StyleProperty::CanvasPaddingRight => VIEM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT,
        StyleProperty::CanvasPaddingBottom => VIEM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM,
        StyleProperty::CanvasPaddingLeft => VIEM_STYLE_PROPERTY_CANVAS_PADDING_LEFT,
        StyleProperty::BlockMarginTop => VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP,
        StyleProperty::BlockMarginBottom => VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM,
        StyleProperty::ParagraphLineSpacing => VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING,
        StyleProperty::ParagraphFirstLineIndent => VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT,
        StyleProperty::ParagraphLeadingIndent => VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT,
        StyleProperty::ParagraphTrailingIndent => VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT,
        StyleProperty::ParagraphAlignment => VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT,
        StyleProperty::ParagraphBaseDirection => VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION,
        StyleProperty::CharacterFontFamilies => VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES,
        StyleProperty::CharacterSize => VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
        StyleProperty::CharacterWeight => VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT,
        StyleProperty::CharacterBold => VIEM_STYLE_PROPERTY_CHARACTER_BOLD,
        StyleProperty::CharacterSlant => VIEM_STYLE_PROPERTY_CHARACTER_SLANT,
        StyleProperty::CharacterForeground => VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND,
        StyleProperty::CharacterBackground => VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND,
        StyleProperty::CharacterUnderline => VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
        StyleProperty::CharacterStrikethrough => VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
        StyleProperty::CharacterLanguage => VIEM_STYLE_PROPERTY_CHARACTER_LANGUAGE,
        StyleProperty::CharacterDirection => VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION,
        StyleProperty::CharacterOpenTypeFeatures => {
            VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES
        }
        StyleProperty::CharacterLetterSpacing => VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING,
        StyleProperty::CharacterScriptPosition => VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION,
    }
}

fn push_style_string(
    strings: &mut Vec<u8>,
    value: &str,
) -> Result<ViemStyleStringRefV1, ViemStatus> {
    let offset = checked_export_count(strings.len())?;
    let length = checked_export_count(value.len())?;
    strings
        .try_reserve(value.len())
        .map_err(|_| ViemStatus::ResourceExhausted)?;
    strings.extend_from_slice(value.as_bytes());
    Ok(ViemStyleStringRefV1 { offset, length })
}

fn empty_style_value() -> ViemStyleValueV1 {
    ViemStyleValueV1 {
        struct_size: VIEM_STYLE_VALUE_V1_SIZE,
        ..ViemStyleValueV1::default()
    }
}

fn style_value_to_ffi(
    value: &StylePropertyValue,
    items: &mut Vec<ViemStyleValueItemV1>,
    strings: &mut Vec<u8>,
) -> Result<ViemStyleValueV1, ViemStatus> {
    let mut output = empty_style_value();
    match value {
        StylePropertyValue::Float(value) => {
            output.kind = VIEM_STYLE_VALUE_FLOAT;
            output.number = *value;
        }
        StylePropertyValue::Percentage(value) => {
            output.kind = VIEM_STYLE_VALUE_PERCENTAGE;
            output.enum_value = u32::from(*value);
        }
        StylePropertyValue::ScriptPosition(value) => {
            output.kind = VIEM_STYLE_VALUE_SCRIPT_POSITION;
            output.enum_value = *value as u32;
        }
        StylePropertyValue::FontWeight(value) => {
            output.kind = VIEM_STYLE_VALUE_UNSIGNED;
            output.enum_value = u32::from(*value);
        }
        StylePropertyValue::Boolean(value) => {
            output.kind = VIEM_STYLE_VALUE_BOOLEAN;
            output.enum_value = u32::from(*value);
        }
        StylePropertyValue::Color(value) => {
            output.kind = VIEM_STYLE_VALUE_COLOR;
            output.color = color_to_ffi(*value);
        }
        StylePropertyValue::Text(value) => {
            output.kind = VIEM_STYLE_VALUE_STRING;
            output.string = push_style_string(strings, value)?;
        }
        StylePropertyValue::FontFamilies(values) => {
            output.kind = VIEM_STYLE_VALUE_STRING_LIST;
            output.first_item = checked_export_count(items.len())?;
            items
                .try_reserve(values.len())
                .map_err(|_| ViemStatus::ResourceExhausted)?;
            for value in values {
                items.push(ViemStyleValueItemV1 {
                    struct_size: VIEM_STYLE_VALUE_ITEM_V1_SIZE,
                    kind: VIEM_STYLE_VALUE_ITEM_STRING,
                    string: push_style_string(strings, value)?,
                    unsigned_value: 0,
                    reserved: 0,
                });
            }
            output.item_count = checked_export_count(values.len())?;
        }
        StylePropertyValue::FontSlant(value) => {
            output.kind = VIEM_STYLE_VALUE_FONT_SLANT;
            output.enum_value = match value {
                FontSlant::Upright => VIEM_FONT_SLANT_UPRIGHT,
                FontSlant::Italic => VIEM_FONT_SLANT_ITALIC,
                FontSlant::Oblique => VIEM_FONT_SLANT_OBLIQUE,
            };
        }
        StylePropertyValue::WritingDirection(value) => {
            output.kind = VIEM_STYLE_VALUE_WRITING_DIRECTION;
            output.enum_value = match value {
                WritingDirection::Natural => VIEM_TEXT_DIRECTION_AUTO,
                WritingDirection::LeftToRight => VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                WritingDirection::RightToLeft => VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT,
            };
        }
        StylePropertyValue::OpenTypeFeatures(values) => {
            output.kind = VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES;
            output.first_item = checked_export_count(items.len())?;
            items
                .try_reserve(values.len())
                .map_err(|_| ViemStatus::ResourceExhausted)?;
            for (tag, value) in values {
                items.push(ViemStyleValueItemV1 {
                    struct_size: VIEM_STYLE_VALUE_ITEM_V1_SIZE,
                    kind: VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE,
                    string: push_style_string(strings, tag)?,
                    unsigned_value: *value,
                    reserved: 0,
                });
            }
            output.item_count = checked_export_count(values.len())?;
        }
        StylePropertyValue::LineSpacing(value) => {
            output.kind = VIEM_STYLE_VALUE_LINE_SPACING;
            match value {
                LineSpacing::Normal => output.enum_value = VIEM_STYLE_LINE_SPACING_NORMAL,
                LineSpacing::Multiplier(value) => {
                    output.enum_value = VIEM_STYLE_LINE_SPACING_MULTIPLIER;
                    output.number = *value;
                }
                LineSpacing::AtLeast(value) => {
                    output.enum_value = VIEM_STYLE_LINE_SPACING_AT_LEAST;
                    output.number = *value;
                }
                LineSpacing::Exact(value) => {
                    output.enum_value = VIEM_STYLE_LINE_SPACING_EXACT;
                    output.number = *value;
                }
            }
        }
        StylePropertyValue::ParagraphAlignment(value) => {
            output.kind = VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT;
            output.enum_value = match value {
                ParagraphAlignment::Start => VIEM_STYLE_PARAGRAPH_ALIGNMENT_START,
                ParagraphAlignment::End => VIEM_STYLE_PARAGRAPH_ALIGNMENT_END,
                ParagraphAlignment::Center => VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER,
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
        StyleProperty::CharacterSize => properties.size.map(|size| match size {
            FontSize::Points(value) => StylePropertyValue::Float(value),
            FontSize::Percentage(value) => StylePropertyValue::Percentage(value),
        }),
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
        StyleProperty::CharacterScriptPosition => {
            properties.script_position.map(StylePropertyValue::ScriptPosition)
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
        StyleProperty::BlockMarginRight => block.margin_right.map(StylePropertyValue::Float),
        StyleProperty::BlockMarginLeft => block.margin_left.map(StylePropertyValue::Float),
        StyleProperty::BlockPaddingTop => block.padding_top.map(StylePropertyValue::Float),
        StyleProperty::BlockPaddingRight => block.padding_right.map(StylePropertyValue::Float),
        StyleProperty::BlockPaddingBottom => block.padding_bottom.map(StylePropertyValue::Float),
        StyleProperty::BlockPaddingLeft => block.padding_left.map(StylePropertyValue::Float),
        StyleProperty::BlockBorderTopWidth => block.border_top_width.map(StylePropertyValue::Float),
        StyleProperty::BlockBorderTopColor => block.border_top_color.map(StylePropertyValue::Color),
        StyleProperty::BlockBorderRightWidth => block.border_right_width.map(StylePropertyValue::Float),
        StyleProperty::BlockBorderRightColor => block.border_right_color.map(StylePropertyValue::Color),
        StyleProperty::BlockBorderBottomWidth => block.border_bottom_width.map(StylePropertyValue::Float),
        StyleProperty::BlockBorderBottomColor => block.border_bottom_color.map(StylePropertyValue::Color),
        StyleProperty::BlockBorderLeftWidth => block.border_left_width.map(StylePropertyValue::Float),
        StyleProperty::BlockBorderLeftColor => block.border_left_color.map(StylePropertyValue::Color),
        StyleProperty::BlockBackground => block.background.map(StylePropertyValue::Color),

        StyleProperty::CanvasBackground => block.background.map(StylePropertyValue::Color),
        StyleProperty::CanvasPaddingTop => block.padding_top.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingRight => block.padding_right.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingBottom => block.padding_bottom.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingLeft => block.padding_left.map(StylePropertyValue::Float),
        StyleProperty::BlockMarginTop => {
            block.margin_top.map(StylePropertyValue::Float)
        }
        StyleProperty::BlockMarginBottom => block.margin_bottom.map(StylePropertyValue::Float),
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
        StyleProperty::CharacterScriptPosition => {
            Some(StylePropertyValue::ScriptPosition(properties.script_position))
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
        StyleProperty::BlockMarginRight => Some(StylePropertyValue::Float(resolved.margin_right)),
        StyleProperty::BlockMarginLeft => Some(StylePropertyValue::Float(resolved.margin_left)),
        StyleProperty::BlockPaddingTop => Some(StylePropertyValue::Float(resolved.padding_top)),
        StyleProperty::BlockPaddingRight => Some(StylePropertyValue::Float(resolved.padding_right)),
        StyleProperty::BlockPaddingBottom => Some(StylePropertyValue::Float(resolved.padding_bottom)),
        StyleProperty::BlockPaddingLeft => Some(StylePropertyValue::Float(resolved.padding_left)),
        StyleProperty::BlockBorderTopWidth => Some(StylePropertyValue::Float(resolved.border_top_width)),
        StyleProperty::BlockBorderTopColor => Some(StylePropertyValue::Color(resolved.border_top_color.unwrap_or(resolved.character.foreground))),
        StyleProperty::BlockBorderRightWidth => Some(StylePropertyValue::Float(resolved.border_right_width)),
        StyleProperty::BlockBorderRightColor => Some(StylePropertyValue::Color(resolved.border_right_color.unwrap_or(resolved.character.foreground))),
        StyleProperty::BlockBorderBottomWidth => Some(StylePropertyValue::Float(resolved.border_bottom_width)),
        StyleProperty::BlockBorderBottomColor => Some(StylePropertyValue::Color(resolved.border_bottom_color.unwrap_or(resolved.character.foreground))),
        StyleProperty::BlockBorderLeftWidth => Some(StylePropertyValue::Float(resolved.border_left_width)),
        StyleProperty::BlockBorderLeftColor => Some(StylePropertyValue::Color(resolved.border_left_color.unwrap_or(resolved.character.foreground))),
        StyleProperty::BlockBackground => resolved.background.map(StylePropertyValue::Color),

        StyleProperty::BlockMarginTop => {
            Some(StylePropertyValue::Float(resolved.margin_top))
        }
        StyleProperty::BlockMarginBottom => {
            Some(StylePropertyValue::Float(resolved.margin_bottom))
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
) -> Result<(u32, u32, ViemStyleStringRefV1), ViemStatus> {
    let none = ViemStyleStringRefV1::default();
    Ok(match origin {
        StyleContributionOrigin::EngineEmergency => {
            (VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY, 0, none)
        }
        StyleContributionOrigin::BlockStyle(id) => (
            VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE,
            VIEM_STYLE_NAMESPACE_BLOCK,
            push_style_string(strings, &id.0)?,
        ),
        StyleContributionOrigin::CharacterStyle(id) => (
            VIEM_STYLE_CONTRIBUTOR_CHARACTER_STYLE,
            VIEM_STYLE_NAMESPACE_CHARACTER,
            push_style_string(strings, &id.0)?,
        ),
        StyleContributionOrigin::DirectDocumentCanvas => {
            (VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS, 0, none)
        }
        StyleContributionOrigin::DirectDocumentCharacter => {
            (VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER, 0, none)
        }
        StyleContributionOrigin::DirectParagraph => {
            (VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH, 0, none)
        }
        StyleContributionOrigin::DirectParagraphCharacter => {
            (VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER, 0, none)
        }
        StyleContributionOrigin::DirectCharacter => {
            (VIEM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER, 0, none)
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn push_style_property(
    properties: &mut Vec<ViemStylePropertyV1>,
    value_items: &mut Vec<ViemStyleValueItemV1>,
    dependencies: &mut Vec<ViemStyleDependencyV1>,
    strings: &mut Vec<u8>,
    property: StyleProperty,
    declared: Option<StylePropertyValue>,
    effective: Option<StylePropertyValue>,
    contribution: &StyleContribution,
) -> Result<(), ViemStatus> {
    let mut flags = 0;
    let declared = if let Some(value) = declared.as_ref() {
        flags |= VIEM_STYLE_PROPERTY_DECLARED;
        style_value_to_ffi(value, value_items, strings)?
    } else {
        empty_style_value()
    };
    let effective = if let Some(value) = effective.as_ref() {
        flags |= VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT;
        style_value_to_ffi(value, value_items, strings)?
    } else {
        empty_style_value()
    };
    let (contributor_kind, contributor_namespace, contributor_style_id) =
        style_contributor_to_ffi(&contribution.winner, strings)?;
    if contributor_style_id.length != 0 {
        flags |= VIEM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE;
    }
    let first_dependency = checked_export_count(dependencies.len())?;
    dependencies
        .try_reserve(contribution.dependencies.len())
        .map_err(|_| ViemStatus::ResourceExhausted)?;
    for dependency in &contribution.dependencies {
        let (namespace, id) = match dependency {
            StyleDependency::Block(id) => (VIEM_STYLE_NAMESPACE_BLOCK, id),
            StyleDependency::Character(id) => (VIEM_STYLE_NAMESPACE_CHARACTER, id),
        };
        dependencies.push(ViemStyleDependencyV1 {
            struct_size: VIEM_STYLE_DEPENDENCY_V1_SIZE,
            namespace,
            style_id: push_style_string(strings, &id.0)?,
        });
    }
    properties.push(ViemStylePropertyV1 {
        struct_size: VIEM_STYLE_PROPERTY_V1_SIZE,
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
        VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS | VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME;
    if !is_base {
        capabilities |= VIEM_STYLE_CAPABILITY_EDIT_PARENT | VIEM_STYLE_CAPABILITY_DELETE;
    }
    if !is_base && role == Some(BlockRole::Paragraph) {
        capabilities |= VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE;
    }
    if source_editable
        && origin == StyleDefinitionOrigin::SourceBacked
        && role != Some(BlockRole::Document)
    {
        capabilities |= VIEM_STYLE_CAPABILITY_ASSIGN;
        if !is_base {
            capabilities |= VIEM_STYLE_CAPABILITY_DELETE;
        }
    }
    capabilities
}

fn export_style_sheet(document: &Document) -> Result<StyleSheetExport, ViemStatus> {
    export_style_sheet_snapshot(document.projection().style_sheet(), style_sheet_identity(document), document.format())
}

fn export_style_sheet_snapshot(sheet: &crate::document::StyleSheet, identity: ViemStyleSheetIdentityV1, format: Format) -> Result<StyleSheetExport, ViemStatus> {
    let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
    let mut definitions = Vec::new();
    let mut properties = Vec::new();
    let mut value_items = Vec::new();
    let mut dependencies = Vec::new();
    let mut strings = Vec::new();
    definitions
        .try_reserve(sheet.block_style_count() + sheet.character_style_count())
        .map_err(|_| ViemStatus::ResourceExhausted)?;

    for style in sheet.block_styles().filter(|style| style.role != BlockRole::Document) {
        let metadata = sheet
            .block_style_metadata(&style.id)
            .ok_or(ViemStatus::CoreFailure)?;
        let first_property = checked_export_count(properties.len())?;
        let property_keys = match style.role {
            BlockRole::Document => [
                CANVAS_STYLE_PROPERTIES.as_slice(),
                CHARACTER_STYLE_PROPERTIES.as_slice(),
            ]
            .concat(),
            BlockRole::Paragraph | BlockRole::Quote | BlockRole::CodeBlock | BlockRole::List | BlockRole::ListItem => [
                PARAGRAPH_STYLE_PROPERTIES.as_slice(),
                CHARACTER_STYLE_PROPERTIES.as_slice(),
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
                            .ok_or(ViemStatus::CoreFailure)?,
                    )?;
                }
            }
            BlockRole::Paragraph | BlockRole::Quote | BlockRole::CodeBlock | BlockRole::List | BlockRole::ListItem => {
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
                            .ok_or(ViemStatus::CoreFailure)?,
                    )?;
                }
            }
        }

        let is_base_paragraph = style.id == sheet.base_paragraph;
        let mut flags = if style.id.is_internal_list() || style.id.legacy_list_level().is_some() {
            VIEM_STYLE_DEFINITION_INTERNAL_LIST
        } else {
            0
        };
        let parent_id = if let Some(parent) = &style.based_on {
            flags |= VIEM_STYLE_DEFINITION_HAS_PARENT;
            push_style_string(&mut strings, &parent.0)?
        } else {
            ViemStyleStringRefV1::default()
        };
        let next_style_id = if let Some(next) = &style.next_paragraph_style {
            flags |= VIEM_STYLE_DEFINITION_HAS_NEXT_STYLE;
            push_style_string(&mut strings, &next.0)?
        } else {
            ViemStyleStringRefV1::default()
        };
        if is_base_paragraph {
            flags |= VIEM_STYLE_DEFINITION_BASE_PARAGRAPH;
        }
        definitions.push(ViemStyleDefinitionV1 {
            struct_size: VIEM_STYLE_DEFINITION_V1_SIZE,
            flags,
            namespace: VIEM_STYLE_NAMESPACE_BLOCK,
            role: style_role_to_ffi(style.role),
            origin: style_origin_to_ffi(metadata.origin),
            capabilities: (generated_style_capabilities(
                metadata.origin,
                is_base_paragraph,
                Some(style.role),
                format.has_rich_source(),
            ) & if format.is_code() { !VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE } else { u32::MAX }) | if sheet.has_user_default(&style.id, false)
                && style.role == BlockRole::Paragraph
                && ((format == Format::Rtf && style.id.0.starts_with("RtfP")))
            {
                VIEM_STYLE_CAPABILITY_ASSIGN
            } else {
                0
            } | if format.is_markdown()
                && matches!(style.id.0.as_str(), "Block quote" | "Code Block")
            {
                VIEM_STYLE_CAPABILITY_ASSIGN
            } else {
                0
            } | if format == Format::Rtf
                && style.id.0.starts_with("List")
                && crate::document::StyleSheet::builtin_block(&style.id)
            {
                VIEM_STYLE_CAPABILITY_ASSIGN
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
            .ok_or(ViemStatus::CoreFailure)?;
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
        for property in CHARACTER_STYLE_PROPERTIES {
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
                    .ok_or(ViemStatus::CoreFailure)?,
            )?;
        }
        let is_base = false;
        let mut flags = 0;
        if style.id.is_internal() {
            flags |= VIEM_STYLE_DEFINITION_INTERNAL;
        }
        if sheet.is_implicit_character(&style.id) {
            flags |= VIEM_STYLE_DEFINITION_IMPLICIT;
        }
        let parent_id = if let Some(parent) = &style.based_on {
            flags |= VIEM_STYLE_DEFINITION_HAS_PARENT;
            push_style_string(&mut strings, &parent.0)?
        } else {
            ViemStyleStringRefV1::default()
        };
        definitions.push(ViemStyleDefinitionV1 {
            struct_size: VIEM_STYLE_DEFINITION_V1_SIZE,
            flags,
            namespace: VIEM_STYLE_NAMESPACE_CHARACTER,
            role: VIEM_STYLE_ROLE_NONE,
            origin: style_origin_to_ffi(metadata.origin),
            capabilities: if style.id.is_internal() {
                VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS
            } else {
                generated_style_capabilities(
                    metadata.origin,
                    is_base,
                    None,
                    format.has_rich_source(),
                ) | if format.has_rich_source() || format.is_markdown() && style.id.0 == "Code"
                {
                    VIEM_STYLE_CAPABILITY_ASSIGN
                } else {
                    0
                }
            },
            stable_id: push_style_string(&mut strings, &style.id.0)?,
            display_name: push_style_string(&mut strings, &metadata.display_name)?,
            parent_id,
            next_style_id: ViemStyleStringRefV1::default(),
            first_property,
            property_count: checked_export_count(CHARACTER_STYLE_PROPERTIES.len())?,
        });
    }

    let info = ViemStyleSheetInfoV1 {
        struct_size: VIEM_STYLE_SHEET_INFO_V1_SIZE,
        reserved: 0,
        identity,
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
) -> Result<ViemLayoutPaintInfoV1, ViemStatus> {
    Ok(ViemLayoutPaintInfoV1 {
        struct_size: VIEM_LAYOUT_PAINT_INFO_V1_SIZE,
        flags: if snapshot.canvas_background_is_default {
            VIEM_LAYOUT_PAINT_DEFAULT_CANVAS
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
) -> Result<LayoutPaintExport, ViemStatus> {
    let info = layout_paint_info(snapshot, view_id)?;
    let mut previous_end = None;
    let runs = snapshot
        .paint_runs
        .iter()
        .map(|run| {
            if run.text_range.start > run.text_range.end
                || previous_end.is_some_and(|end| run.text_range.start < end)
            {
                return Err(ViemStatus::CoreFailure);
            }
            previous_end = Some(run.text_range.end);
            Ok(ViemPaintStyleRunV1 {
                struct_size: VIEM_PAINT_STYLE_RUN_V1_SIZE,
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
    info: ViemCommandLineInfoV1,
    bytes: Vec<u8>,
}

struct VisualSelectionExport {
    info: ViemVisualSelectionInfoV1,
    segments: Vec<ViemVisualSelectionSegmentV1>,
    rectangles: Vec<ViemVisualSelectionRectangleV1>,
}

fn command_line_kind_to_ffi(kind: Option<CommandLineKind>) -> u32 {
    match kind {
        None => VIEM_COMMAND_LINE_KIND_NONE,
        Some(CommandLineKind::Ex) => VIEM_COMMAND_LINE_KIND_EX,
        Some(CommandLineKind::SearchForward) => VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD,
        Some(CommandLineKind::SearchBackward) => VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD,
    }
}

fn command_line_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        VIEM_COMMAND_LINE_KIND_NONE
            | VIEM_COMMAND_LINE_KIND_EX
            | VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD
            | VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD
    )
}

fn visual_selection_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        VIEM_VISUAL_SELECTION_KIND_NONE
            | VIEM_VISUAL_SELECTION_KIND_CHARACTER
            | VIEM_VISUAL_SELECTION_KIND_LINE
            | VIEM_VISUAL_SELECTION_KIND_BLOCK
    )
}

fn opaque_state_identity(material: &[u8]) -> [u8; 32] {
    *SourceArtifactDigest::from_bytes(material).as_bytes()
}

fn export_command_line(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<CommandLineExport, ViemStatus> {
    let state = core.command_state(view_id).ok_or(ViemStatus::InvalidView)?;
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
        (None, None, None) => (VIEM_COMMAND_LINE_KIND_NONE, Vec::new(), 0),
        _ => return Err(ViemStatus::CoreFailure),
    };
    let utf8_length = checked_export_count(bytes.len())?;
    let cursor_utf8_offset = checked_export_count(cursor)?;
    let mut material = Vec::with_capacity(32usize.saturating_add(bytes.len()));
    material.extend_from_slice(b"viem-command-line-v1\0");
    material.extend_from_slice(&kind.to_le_bytes());
    material.extend_from_slice(&utf8_length.to_le_bytes());
    material.extend_from_slice(&cursor_utf8_offset.to_le_bytes());
    let anchor = state
        .command_line_snapshot()
        .map_or(cursor, |snapshot| snapshot.anchor);
    material.extend_from_slice(&(anchor as u64).to_le_bytes());
    material.extend_from_slice(&bytes);
    let identity = ViemCommandLineIdentityV1 {
        struct_size: VIEM_COMMAND_LINE_IDENTITY_V1_SIZE,
        kind,
        view_id: view_id.0,
        document_id: core.document().id().0,
        document_revision: core.document().revision().0,
        state_identity: opaque_state_identity(&material),
    };
    Ok(CommandLineExport {
        info: ViemCommandLineInfoV1 {
            struct_size: VIEM_COMMAND_LINE_INFO_V1_SIZE,
            reserved: 0,
            identity,
            utf8_length,
            cursor_utf8_offset,
        },
        bytes,
    })
}

fn validate_command_line_identity(
    expected: ViemCommandLineIdentityV1,
    actual: ViemCommandLineIdentityV1,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_COMMAND_LINE_IDENTITY_V1_SIZE
        || !command_line_kind_is_valid(expected.kind)
    {
        return Err(ViemStatus::InvalidArgument);
    }
    (expected.kind == actual.kind
        && expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn visual_block_status(error: VisualBlockError) -> ViemStatus {
    match error {
        VisualBlockError::WrongDocument { .. }
        | VisualBlockError::WrongDocumentRevision { .. }
        | VisualBlockError::StaleLayout { .. } => ViemStatus::StaleRevision,
        VisualBlockError::EndpointNotInLayout(_) | VisualBlockError::EmptyLayout | VisualBlockError::NeedsLayout(_) => {
            ViemStatus::OutsideLayoutCoverage
        }
        VisualBlockError::NonGraphemeBoundary(_) => ViemStatus::NotGraphemeBoundary,
        VisualBlockError::InvalidX => ViemStatus::InvalidArgument,
        VisualBlockError::TextDoesNotMatchLayout
        | VisualBlockError::OverlappingRanges
        | VisualBlockError::ReplacementTooLarge { .. } => ViemStatus::CoreFailure,
    }
}

fn checked_selection_range(
    document: &Document,
    start: usize,
    end: usize,
) -> Result<TextRange, ViemStatus> {
    let start = document.text_point(start).map_err(document_status)?;
    let end = document.text_point(end).map_err(document_status)?;
    TextRange::new(start, end).map_err(|_| ViemStatus::CoreFailure)
}

fn push_selection_rectangles(
    snapshot: &LayoutSnapshot,
    range: TextRange,
    empty_affinity: BoundaryAffinity,
    segment_index: usize,
    expected_row: Option<usize>,
    rectangles: &mut Vec<ViemVisualSelectionRectangleV1>,
) -> Result<(), ViemStatus> {
    let resolved = if expected_row.is_some() {
        // Block rows are explicitly resolved against this exact layout.
        snapshot.selection_rectangles(range, empty_affinity)
    } else {
        // Linear selection endpoints may have scrolled out of the regional
        // snapshot. Keep their complete logical segment and export the exact
        // geometry that is materialized, including selected hard breaks.
        snapshot.materialized_selection_rectangles(range, empty_affinity)
    }
    .map_err(layout_query_status)?;
    let mut matched = false;
    for rectangle in resolved {
        if expected_row.is_some_and(|row| row != rectangle.row_index) {
            continue;
        }
        matched = true;
        rectangles.push(ViemVisualSelectionRectangleV1 {
            struct_size: VIEM_VISUAL_SELECTION_RECTANGLE_V1_SIZE,
            reserved: 0,
            row_index: checked_export_count(rectangle.row_index)?,
            segment_index: checked_export_count(segment_index)?,
            rect: layout_rect_to_ffi(rectangle.rect),
        });
    }
    if expected_row.is_some() && !matched {
        return Err(ViemStatus::CoreFailure);
    }
    Ok(())
}

fn visual_selection_state_identity(
    kind: u32,
    segments: &[ViemVisualSelectionSegmentV1],
    rectangles: &[ViemVisualSelectionRectangleV1],
) -> [u8; 32] {
    let mut material = Vec::with_capacity(
        40usize
            .saturating_add(segments.len().saturating_mul(48))
            .saturating_add(rectangles.len().saturating_mul(40)),
    );
    material.extend_from_slice(b"viem-visual-selection-v1\0");
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
) -> Result<VisualSelectionExport, ViemStatus> {
    let snapshot = current_ffi_layout_snapshot(core, view_id)?;
    let state = core.command_state(view_id).ok_or(ViemStatus::InvalidView)?;
    let document = core.document();
    let mut segments = Vec::new();
    let mut rectangles = Vec::new();
    let kind = match state.mode() {
        Mode::VisualCharacter | Mode::VisualLine => {
            let kind = if state.mode() == Mode::VisualCharacter {
                VIEM_VISUAL_SELECTION_KIND_CHARACTER
            } else {
                VIEM_VISUAL_SELECTION_KIND_LINE
            };
            let range = state
                .line_selection_range(document, Some(snapshot))
                .ok_or(ViemStatus::CoreFailure)?;
            let checked = checked_selection_range(document, range.start, range.end)?;
            segments.push(ViemVisualSelectionSegmentV1 {
                struct_size: VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE,
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
            let selection = state.visual_block().ok_or(ViemStatus::CoreFailure)?;
            let resolved = if state.visual_block_to_line_end() {
                resolve_block_selection_to_line_end(selection, snapshot, document.text())
            } else {
                resolve_block_selection(selection, snapshot, document.text())
            }
            .map_err(visual_block_status)?;
            for (segment_index, segment) in resolved.range_set.segments.iter().enumerate() {
                let checked =
                    checked_selection_range(document, segment.range.start, segment.range.end)?;
                segments.push(ViemVisualSelectionSegmentV1 {
                    struct_size: VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE,
                    flags: VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW
                        | VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE
                        | VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES,
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
            VIEM_VISUAL_SELECTION_KIND_BLOCK
        }
        Mode::Normal | Mode::Insert | Mode::Replace | Mode::CommandLine => {
            VIEM_VISUAL_SELECTION_KIND_NONE
        }
    };
    let identity = ViemVisualSelectionIdentityV1 {
        struct_size: VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE,
        kind,
        layout: snapshot_identity(snapshot, view_id),
        state_identity: visual_selection_state_identity(kind, &segments, &rectangles),
    };
    Ok(VisualSelectionExport {
        info: ViemVisualSelectionInfoV1 {
            struct_size: VIEM_VISUAL_SELECTION_INFO_V1_SIZE,
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
) -> Result<String, ViemStatus> {
    if export.info.identity.kind == VIEM_VISUAL_SELECTION_KIND_NONE || export.segments.is_empty() {
        return Err(ViemStatus::InvalidRange);
    }
    let block = export.info.identity.kind == VIEM_VISUAL_SELECTION_KIND_BLOCK;
    let mut text = String::new();
    for (index, segment) in export.segments.iter().enumerate() {
        let start = checked_length(segment.text_start)?;
        let end = checked_length(segment.text_end)?;
        let piece = document
            .text()
            .get(start..end)
            .ok_or(ViemStatus::InvalidRange)?;
        if block && index != 0 {
            text.push('\n');
        }
        text.push_str(piece);
    }
    if text.is_empty() {
        return Err(ViemStatus::InvalidRange);
    }
    Ok(text)
}

fn validate_visual_selection_identity(
    expected: ViemVisualSelectionIdentityV1,
    actual: ViemVisualSelectionIdentityV1,
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE
        || !visual_selection_kind_is_valid(expected.kind)
    {
        return Err(ViemStatus::InvalidArgument);
    }
    validate_snapshot_identity(expected.layout, snapshot, view_id)?;
    (expected.kind == actual.kind && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn logical_selection_kind_to_ffi(kind: LogicalSelectionKind) -> u32 {
    match kind {
        LogicalSelectionKind::None => VIEM_LOGICAL_SELECTION_KIND_NONE,
        LogicalSelectionKind::Character => VIEM_LOGICAL_SELECTION_KIND_CHARACTER,
        LogicalSelectionKind::Line => VIEM_LOGICAL_SELECTION_KIND_LINE,
        LogicalSelectionKind::Block => VIEM_LOGICAL_SELECTION_KIND_BLOCK,
    }
}

fn logical_selection_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        VIEM_LOGICAL_SELECTION_KIND_NONE
            | VIEM_LOGICAL_SELECTION_KIND_CHARACTER
            | VIEM_LOGICAL_SELECTION_KIND_LINE
            | VIEM_LOGICAL_SELECTION_KIND_BLOCK
    )
}

fn semantic_style_from_ffi(style: u32) -> Result<SemanticInlineStyle, ViemStatus> {
    match style {
        VIEM_SEMANTIC_STYLE_STRONG => Ok(SemanticInlineStyle::Strong),
        VIEM_SEMANTIC_STYLE_EMPHASIS => Ok(SemanticInlineStyle::Emphasis),
        _ => Err(ViemStatus::InvalidArgument),
    }
}

fn semantic_style_to_ffi(style: SemanticInlineStyle) -> Result<u32, ViemStatus> {
    match style {
        SemanticInlineStyle::Strong => Ok(VIEM_SEMANTIC_STYLE_STRONG),
        SemanticInlineStyle::Emphasis => Ok(VIEM_SEMANTIC_STYLE_EMPHASIS),
        SemanticInlineStyle::Code => Err(ViemStatus::InvalidArgument),
    }
}

fn semantic_style_state_to_ffi(state: SemanticStyleState) -> u32 {
    match state {
        SemanticStyleState::Off => VIEM_SEMANTIC_STYLE_STATE_OFF,
        SemanticStyleState::On => VIEM_SEMANTIC_STYLE_STATE_ON,
        SemanticStyleState::Mixed => VIEM_SEMANTIC_STYLE_STATE_MIXED,
    }
}

fn logical_selection_state_identity(selection: &LogicalSelectionIdentity) -> [u8; 32] {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(b"viem-logical-selection-v1\0");
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
) -> Result<ViemLogicalSelectionIdentityV1, ViemStatus> {
    let range = selection.range();
    Ok(ViemLogicalSelectionIdentityV1 {
        struct_size: VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE,
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
) -> Result<(ViemSemanticStylePresentationV1, SemanticStylePresentation), ViemStatus> {
    let presentation = core
        .selection_semantic_style_presentation(view_id, style)
        .map_err(core_status)?;
    let selection = match presentation.selection() {
        Some(selection) => logical_selection_identity_to_ffi(selection)?,
        None => ViemLogicalSelectionIdentityV1 {
            struct_size: VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE,
            kind: logical_selection_kind_to_ffi(presentation.selection_kind()),
            view_id: view_id.0,
            document_id: core.document().id().0,
            document_revision: core.document().revision().0,
            ..ViemLogicalSelectionIdentityV1::default()
        },
    };
    let mut flags = 0;
    if presentation
        .selection()
        .is_some_and(|selection| !selection.range().is_empty())
    {
        flags |= VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE;
    }
    if presentation
        .selection()
        .is_some_and(|selection| selection.kind() == LogicalSelectionKind::None)
    {
        flags |= VIEM_SEMANTIC_STYLE_TYPING_CONTEXT;
    }
    if presentation.can_set() {
        flags |= VIEM_SEMANTIC_STYLE_CAN_SET;
    }
    if presentation.can_clear() {
        flags |= VIEM_SEMANTIC_STYLE_CAN_CLEAR;
    }
    Ok((
        ViemSemanticStylePresentationV1 {
            struct_size: VIEM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE,
            style: semantic_style_to_ffi(style)?,
            state: semantic_style_state_to_ffi(presentation.state()),
            flags,
            selection,
        },
        presentation,
    ))
}

fn validate_logical_selection_identity(
    expected: ViemLogicalSelectionIdentityV1,
    actual: ViemLogicalSelectionIdentityV1,
) -> Result<(), ViemStatus> {
    if expected.struct_size < VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE
        || !logical_selection_kind_is_valid(expected.kind)
    {
        return Err(ViemStatus::InvalidArgument);
    }
    (expected.kind == actual.kind
        && expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.text_start == actual.text_start
        && expected.text_end == actual.text_end
        && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(ViemStatus::StaleRevision)
}

fn export_layout_snapshot(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<LayoutSnapshotExport, ViemStatus> {
    let info = layout_snapshot_info(snapshot, view_id)?;
    let row_capacity = usize::try_from(info.row_count).map_err(|_| ViemStatus::LengthOverflow)?;
    let cluster_capacity =
        usize::try_from(info.cluster_count).map_err(|_| ViemStatus::LengthOverflow)?;
    let caret_capacity =
        usize::try_from(info.caret_count).map_err(|_| ViemStatus::LengthOverflow)?;
    let mut rows = Vec::with_capacity(row_capacity);
    let mut clusters = Vec::with_capacity(cluster_capacity);
    let mut carets = Vec::with_capacity(caret_capacity);
    for (row_index, row) in snapshot.rows.iter().enumerate() {
        let first_cluster = checked_export_count(clusters.len())?;
        let first_caret = checked_export_count(carets.len())?;
        for cluster in &row.clusters {
            let (flags, render_run) = cluster.render_run.as_ref().map_or_else(
                || (0, ViemRenderRunHandleV1::default()),
                |render_run| {
                    (
                        VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN,
                        render_run_to_ffi(render_run),
                    )
                },
            );
            clusters.push(ViemPositionedClusterV1 {
                struct_size: VIEM_POSITIONED_CLUSTER_V1_SIZE,
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
            carets.push(ViemPositionedCaretV1 {
                struct_size: VIEM_POSITIONED_CARET_V1_SIZE,
                affinity: affinity_to_ffi(caret.point.affinity),
                row_index: checked_export_count(row_index)?,
                text_offset: checked_export_count(caret.point.text_offset)?,
                x: caret.x,
                reserved: 0.0,
            });
        }
        let mut flags = 0;
        let paragraph_id = row.paragraph_id.map_or(0, |paragraph_id| {
            flags |= VIEM_VISUAL_ROW_HAS_PARAGRAPH;
            paragraph_id
        });
        if row.wrapped_from_previous {
            flags |= VIEM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS;
        }
        if row.wraps_to_next {
            flags |= VIEM_VISUAL_ROW_WRAPS_TO_NEXT;
        }
        rows.push(ViemVisualRowV1 {
            struct_size: VIEM_VISUAL_ROW_V1_SIZE,
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
) -> Result<ViemViewPresentationV1, ViemStatus> {
    let state = core.command_state(view_id).ok_or(ViemStatus::InvalidView)?;
    // One authority decides what the caret occupies. The offset and affinity
    // below are exported from the same target, so they cannot disagree with
    // the cell a frontend draws.
    let caret = state.caret_target(core.document());
    let mut flags = 0;
    if state.literal_input_pending() {
        flags |= VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING;
    }
    if state.command_line_register_pending() {
        flags |= VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING;
    }
    let mut cursor_offset = state.cursor();
    let mut cursor_affinity = state.boundary_affinity();
    let mut visual_anchor_offset = 0;
    let mut visual_anchor_affinity = 0;
    let mut block_left_x = 0.0;
    let mut block_right_x = 0.0;
    if let Some(block) = state.visual_block() {
        flags |= VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR
            | VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT
            | VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK;
        cursor_offset = block.active.text_offset;
        cursor_affinity = block.active.affinity;
        visual_anchor_offset = checked_export_count(block.anchor.text_offset)?;
        visual_anchor_affinity = affinity_to_ffi(block.anchor.affinity);
        block_left_x = block.left_x();
        block_right_x = block.right_x();
    } else if let Some(anchor) = state.visual_anchor() {
        flags |= VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR;
        visual_anchor_offset = checked_export_count(anchor)?;
    }
    let desired_x = state.desired_x().map_or(0.0, |desired_x| {
        flags |= VIEM_VIEW_PRESENTATION_HAS_DESIRED_X;
        desired_x
    });
    let (command_line_length, command_line_cursor) =
        if let Some(command_line) = state.command_line() {
            flags |= VIEM_VIEW_PRESENTATION_HAS_COMMAND_LINE;
            (
                checked_export_count(command_line.len())?,
                checked_export_count(state.command_line_cursor().unwrap_or(0))?,
            )
        } else {
            (0, 0)
        };
    Ok(ViemViewPresentationV1 {
        struct_size: VIEM_VIEW_PRESENTATION_V1_SIZE,
        flags,
        mode: mode_to_ffi(state.mode(), state.is_select_mode(), state.is_native_selection()),
        cursor_affinity: affinity_to_ffi(cursor_affinity),
        visual_anchor_affinity,
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
        caret_shape: if caret.is_cell() {
            VIEM_CARET_SHAPE_CELL
        } else {
            VIEM_CARET_SHAPE_BOUNDARY
        },
        caret_utf8_start: checked_export_count(caret.range().start)?,
        caret_utf8_end: checked_export_count(caret.range().end)?,
    })
}

fn typed_pointer_region<T>(pointer: *const T, count: u64) -> Result<(usize, usize), ViemStatus> {
    let count = checked_length(count)?;
    if count == 0 {
        return Ok((pointer as usize, 0));
    }
    if pointer.is_null() {
        return Err(ViemStatus::NullPointer);
    }
    if (pointer as usize) % align_of::<T>() != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    let bytes = count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or(ViemStatus::LengthOverflow)?;
    Ok((pointer as usize, bytes))
}

fn regions_overlap(left: (usize, usize), right: (usize, usize)) -> bool {
    pointer_ranges_overlap(left.0 as *const u8, left.1, right.0 as *const u8, right.1)
}

/// Validate a batch before reading requests or writing any of its outputs.
fn validate_disjoint_regions(regions: &[(usize, usize)]) -> Result<(), ViemStatus> {
    for (index, left) in regions.iter().enumerate() {
        if regions[index + 1..]
            .iter()
            .any(|right| regions_overlap(*left, *right))
        {
            return Err(ViemStatus::InvalidArgument);
        }
    }
    Ok(())
}

/// Copy an already validated output, allowing null buffers for empty exports.
///
/// # Safety
/// For a nonempty slice, `output` must be aligned, writable for every element,
/// and disjoint from `values`. Callers must validate every buffer's capacity
/// before copying any member of an atomic batch.
unsafe fn copy_output<T: Copy>(values: &[T], output: *mut T) {
    if !values.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), output, values.len()) };
    }
}

fn parse_clipboard_target(value: u32) -> Result<ClipboardTarget, ViemStatus> {
    match value {
        VIEM_CLIPBOARD_TARGET_CLIPBOARD => Ok(ClipboardTarget::Clipboard),
        VIEM_CLIPBOARD_TARGET_PRIMARY => Ok(ClipboardTarget::Primary),
        _ => Err(ViemStatus::InvalidArgument),
    }
}

unsafe fn read_layout_identity(
    pointer: *const ViemLayoutSnapshotIdentityV1,
) -> Result<ViemLayoutSnapshotIdentityV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_formatted_snapshot_identity(
    pointer: *const ViemFormattedSnapshotIdentityV1,
) -> Result<ViemFormattedSnapshotIdentityV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_style_sheet_identity(
    pointer: *const ViemStyleSheetIdentityV1,
) -> Result<ViemStyleSheetIdentityV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    let identity = unsafe { pointer.read() };
    if identity.struct_size < VIEM_STYLE_SHEET_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_style_edit_group(
    pointer: *const ViemStyleEditGroupV1,
) -> Result<ViemStyleEditGroupV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    let group = unsafe { pointer.read() };
    if group.struct_size < VIEM_STYLE_EDIT_GROUP_V1_SIZE
        || group.flags != 0
        || group.token == 0
        || group.view_id == 0
        || group.document_id == 0
    {
        return Err(ViemStatus::InvalidStyleEditGroup);
    }
    Ok(group)
}

unsafe fn read_command_line_identity(
    pointer: *const ViemCommandLineIdentityV1,
) -> Result<ViemCommandLineIdentityV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < VIEM_COMMAND_LINE_IDENTITY_V1_SIZE
        || !command_line_kind_is_valid(identity.kind)
    {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_visual_selection_identity(
    pointer: *const ViemVisualSelectionIdentityV1,
) -> Result<ViemVisualSelectionIdentityV1, ViemStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE
        || !visual_selection_kind_is_valid(identity.kind)
        || identity.layout.struct_size < VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
        || identity.layout.reserved != 0
    {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn clear_outcome(output: *mut ViemCoreOutcomeV1) -> Result<(), ViemStatus> {
    if output.is_null() {
        return Err(ViemStatus::NullPointer);
    }
    if (output as usize) % align_of::<ViemCoreOutcomeV1>() != 0 {
        return Err(ViemStatus::InvalidArgument);
    }
    // SAFETY: Null and alignment were checked; the public function contract
    // requires one writable output value.
    unsafe { output.write(ViemCoreOutcomeV1::default()) };
    Ok(())
}

/// Copy one fixed-layout request only after validating it cannot be corrupted
/// when the outcome is cleared.
///
/// # Safety
///
/// `request` and `out_outcome` must satisfy the public function's readable and
/// writable pointer contracts respectively.
unsafe fn read_core_request<T: Copy, O>(
    request: *const T,
    out_outcome: *mut O,
) -> Result<T, ViemStatus> {
    if request.is_null() || out_outcome.is_null() {
        return Err(ViemStatus::NullPointer);
    }
    if (request as usize) % align_of::<T>() != 0
        || (out_outcome as usize) % align_of::<O>() != 0
        || pointer_ranges_overlap(
            request.cast(),
            size_of::<T>(),
            out_outcome.cast(),
            size_of::<O>(),
        )
    {
        return Err(ViemStatus::InvalidArgument);
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
unsafe fn composition_utf8<O>(
    value: ViemUtf8Slice,
    out_outcome: *mut O,
) -> Result<String, ViemStatus> {
    let length = checked_length(value.length)?;
    if value.data.is_null() && length != 0 {
        return Err(ViemStatus::NullPointer);
    }
    if pointer_ranges_overlap(
        value.data,
        length,
        out_outcome.cast(),
        size_of::<O>(),
    ) {
        return Err(ViemStatus::InvalidArgument);
    }
    let bytes = unsafe { input_bytes(value.data, value.length)? };
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| ViemStatus::InvalidUtf8)
}

#[derive(Debug)]
struct ParsedStyleEdit {
    identity: ViemStyleSheetIdentityV1,
    namespace: StyleNamespace,
    style: StyleId,
    edit: StyleDefinitionFieldEdit,
}

fn parse_style_namespace(raw: u32) -> Result<StyleNamespace, ViemStatus> {
    match raw {
        VIEM_STYLE_NAMESPACE_BLOCK => Ok(StyleNamespace::Block),
        VIEM_STYLE_NAMESPACE_CHARACTER => Ok(StyleNamespace::Character),
        _ => Err(ViemStatus::InvalidArgument),
    }
}

fn parse_style_property(raw: u32) -> Result<StyleProperty, ViemStatus> {
    match raw {
        VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT => Ok(StyleProperty::BlockMarginRight),
        VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT => Ok(StyleProperty::BlockMarginLeft),
        VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP => Ok(StyleProperty::BlockPaddingTop),
        VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT => Ok(StyleProperty::BlockPaddingRight),
        VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM => Ok(StyleProperty::BlockPaddingBottom),
        VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT => Ok(StyleProperty::BlockPaddingLeft),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH => Ok(StyleProperty::BlockBorderTopWidth),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR => Ok(StyleProperty::BlockBorderTopColor),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH => Ok(StyleProperty::BlockBorderRightWidth),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR => Ok(StyleProperty::BlockBorderRightColor),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH => Ok(StyleProperty::BlockBorderBottomWidth),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR => Ok(StyleProperty::BlockBorderBottomColor),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH => Ok(StyleProperty::BlockBorderLeftWidth),
        VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR => Ok(StyleProperty::BlockBorderLeftColor),
        VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND => Ok(StyleProperty::BlockBackground),
        VIEM_STYLE_PROPERTY_CANVAS_BACKGROUND => Ok(StyleProperty::CanvasBackground),
        VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP => Ok(StyleProperty::CanvasPaddingTop),
        VIEM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT => Ok(StyleProperty::CanvasPaddingRight),
        VIEM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM => Ok(StyleProperty::CanvasPaddingBottom),
        VIEM_STYLE_PROPERTY_CANVAS_PADDING_LEFT => Ok(StyleProperty::CanvasPaddingLeft),
        VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP => Ok(StyleProperty::BlockMarginTop),
        VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM => Ok(StyleProperty::BlockMarginBottom),
        VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING => Ok(StyleProperty::ParagraphLineSpacing),
        VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT => {
            Ok(StyleProperty::ParagraphFirstLineIndent)
        }
        VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT => Ok(StyleProperty::ParagraphLeadingIndent),
        VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT => Ok(StyleProperty::ParagraphTrailingIndent),
        VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT => Ok(StyleProperty::ParagraphAlignment),
        VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION => Ok(StyleProperty::ParagraphBaseDirection),
        VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES => Ok(StyleProperty::CharacterFontFamilies),
        VIEM_STYLE_PROPERTY_CHARACTER_SIZE => Ok(StyleProperty::CharacterSize),
        VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT => Ok(StyleProperty::CharacterWeight),
        VIEM_STYLE_PROPERTY_CHARACTER_BOLD => Ok(StyleProperty::CharacterBold),
        VIEM_STYLE_PROPERTY_CHARACTER_SLANT => Ok(StyleProperty::CharacterSlant),
        VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND => Ok(StyleProperty::CharacterForeground),
        VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND => Ok(StyleProperty::CharacterBackground),
        VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE => Ok(StyleProperty::CharacterUnderline),
        VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH => Ok(StyleProperty::CharacterStrikethrough),
        VIEM_STYLE_PROPERTY_CHARACTER_LANGUAGE => Ok(StyleProperty::CharacterLanguage),
        VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION => Ok(StyleProperty::CharacterDirection),
        VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES => {
            Ok(StyleProperty::CharacterOpenTypeFeatures)
        }
        VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING => Ok(StyleProperty::CharacterLetterSpacing),
        VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION => Ok(StyleProperty::CharacterScriptPosition),
        _ => Err(ViemStatus::InvalidStyleValue),
    }
}

fn style_edit_value_has_no_array(value: &ViemStyleEditValueV1) -> Result<(), ViemStatus> {
    if value.item_count == 0 {
        Ok(())
    } else {
        Err(ViemStatus::InvalidStyleValue)
    }
}

fn style_edit_value_has_no_text(value: &ViemStyleEditValueV1) -> Result<(), ViemStatus> {
    if value.text.length == 0 {
        Ok(())
    } else {
        Err(ViemStatus::InvalidStyleValue)
    }
}

unsafe fn parse_style_edit_items<O>(
    value: &ViemStyleEditValueV1,
    out_outcome: *mut O,
) -> Result<Vec<(u32, String, u32)>, ViemStatus> {
    let region = typed_pointer_region(value.items, value.item_count)?;
    let output_region = typed_pointer_region(out_outcome, 1)?;
    if regions_overlap(region, output_region) {
        return Err(ViemStatus::InvalidArgument);
    }
    let count = checked_length(value.item_count)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let raw_items = unsafe { slice::from_raw_parts(value.items, count) };
    let mut parsed = Vec::new();
    parsed
        .try_reserve(count)
        .map_err(|_| ViemStatus::ResourceExhausted)?;
    for item in raw_items {
        if item.struct_size < VIEM_STYLE_EDIT_VALUE_ITEM_V1_SIZE || item.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        parsed.push((
            item.kind,
            unsafe { composition_utf8(item.text, out_outcome)? },
            item.unsigned_value,
        ));
    }
    Ok(parsed)
}

// Relative font sizes are named-style declarations. Native direct-formatting
// controls continue to exchange absolute sizes.
unsafe fn parse_direct_style_property_value<O>(
    property: StyleProperty,
    value: &ViemStyleEditValueV1,
    out_outcome: *mut O,
) -> Result<StylePropertyValue, ViemStatus> {
    let value = unsafe { parse_style_property_value(property, value, out_outcome)? };
    if matches!(value, StylePropertyValue::Percentage(_)) {
        return Err(ViemStatus::InvalidStyleValue);
    }
    Ok(value)
}

unsafe fn parse_style_property_value<O>(
    property: StyleProperty,
    value: &ViemStyleEditValueV1,
    out_outcome: *mut O,
) -> Result<StylePropertyValue, ViemStatus> {
    if value.struct_size < VIEM_STYLE_EDIT_VALUE_V1_SIZE
        || value.reserved != 0
        || value.number_reserved != 0.0
    {
        return Err(ViemStatus::InvalidArgument);
    }
    let invalid = || ViemStatus::InvalidStyleValue;
    match property {
        StyleProperty::BlockMarginRight
        | StyleProperty::BlockMarginLeft
        | StyleProperty::BlockPaddingTop
        | StyleProperty::BlockPaddingRight
        | StyleProperty::BlockPaddingBottom
        | StyleProperty::BlockPaddingLeft
        | StyleProperty::BlockBorderTopWidth
        | StyleProperty::BlockBorderRightWidth
        | StyleProperty::BlockBorderBottomWidth
        | StyleProperty::BlockBorderLeftWidth
        | StyleProperty::CanvasPaddingTop
        | StyleProperty::CanvasPaddingRight
        | StyleProperty::CanvasPaddingBottom
        | StyleProperty::CanvasPaddingLeft
        | StyleProperty::BlockMarginTop
        | StyleProperty::BlockMarginBottom
        | StyleProperty::ParagraphFirstLineIndent
        | StyleProperty::ParagraphLeadingIndent
        | StyleProperty::ParagraphTrailingIndent
        | StyleProperty::CharacterLetterSpacing => {
            if value.kind != VIEM_STYLE_VALUE_FLOAT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::Float(value.number))
        }
        StyleProperty::CharacterSize => {
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            match value.kind {
                VIEM_STYLE_VALUE_FLOAT => Ok(StylePropertyValue::Float(value.number)),
                VIEM_STYLE_VALUE_PERCENTAGE
                    if (10..=1000).contains(&value.enum_value) && value.number == 0.0 =>
                {
                    Ok(StylePropertyValue::Percentage(value.enum_value as u16))
                }
                _ => Err(invalid()),
            }
        }
        StyleProperty::CharacterScriptPosition => {
            if value.kind != VIEM_STYLE_VALUE_SCRIPT_POSITION { return Err(invalid()); }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::ScriptPosition(match value.enum_value {
                VIEM_SCRIPT_POSITION_NORMAL => ScriptPosition::Normal,
                VIEM_SCRIPT_POSITION_SUPERSCRIPT => ScriptPosition::Superscript,
                VIEM_SCRIPT_POSITION_SUBSCRIPT => ScriptPosition::Subscript,
                _ => return Err(invalid()),
            }))
        }
        StyleProperty::CharacterWeight => {
            if value.kind != VIEM_STYLE_VALUE_UNSIGNED {
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
            if value.kind != VIEM_STYLE_VALUE_BOOLEAN || value.enum_value > 1 {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::Boolean(value.enum_value != 0))
        }
        StyleProperty::BlockBorderTopColor
        | StyleProperty::BlockBorderRightColor
        | StyleProperty::BlockBorderBottomColor
        | StyleProperty::BlockBorderLeftColor
        | StyleProperty::BlockBackground
        | StyleProperty::CanvasBackground
        | StyleProperty::CharacterForeground
        | StyleProperty::CharacterBackground => {
            if value.kind != VIEM_STYLE_VALUE_COLOR {
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
            if value.kind != VIEM_STYLE_VALUE_STRING {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            Ok(StylePropertyValue::Text(unsafe {
                composition_utf8(value.text, out_outcome)?
            }))
        }
        StyleProperty::CharacterFontFamilies => {
            if value.kind != VIEM_STYLE_VALUE_STRING_LIST {
                return Err(invalid());
            }
            style_edit_value_has_no_text(value)?;
            let items = unsafe { parse_style_edit_items(value, out_outcome)? };
            if items.iter().any(|(kind, text, unsigned)| {
                *kind != VIEM_STYLE_VALUE_ITEM_STRING || text.is_empty() || *unsigned != 0
            }) {
                return Err(invalid());
            }
            Ok(StylePropertyValue::FontFamilies(
                items.into_iter().map(|(_, text, _)| text).collect(),
            ))
        }
        StyleProperty::CharacterSlant => {
            if value.kind != VIEM_STYLE_VALUE_FONT_SLANT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let slant = match value.enum_value {
                VIEM_FONT_SLANT_UPRIGHT => FontSlant::Upright,
                VIEM_FONT_SLANT_ITALIC => FontSlant::Italic,
                VIEM_FONT_SLANT_OBLIQUE => FontSlant::Oblique,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::FontSlant(slant))
        }
        StyleProperty::CharacterDirection | StyleProperty::ParagraphBaseDirection => {
            if value.kind != VIEM_STYLE_VALUE_WRITING_DIRECTION {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let direction = match value.enum_value {
                VIEM_TEXT_DIRECTION_AUTO => WritingDirection::Natural,
                VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT => WritingDirection::LeftToRight,
                VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT => WritingDirection::RightToLeft,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::WritingDirection(direction))
        }
        StyleProperty::CharacterOpenTypeFeatures => {
            if value.kind != VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES {
                return Err(invalid());
            }
            style_edit_value_has_no_text(value)?;
            let items = unsafe { parse_style_edit_items(value, out_outcome)? };
            let mut features = std::collections::BTreeMap::new();
            for (kind, tag, setting) in items {
                if kind != VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE
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
            if value.kind != VIEM_STYLE_VALUE_LINE_SPACING {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let spacing = match value.enum_value {
                VIEM_STYLE_LINE_SPACING_NORMAL => LineSpacing::Normal,
                VIEM_STYLE_LINE_SPACING_MULTIPLIER => LineSpacing::Multiplier(value.number),
                VIEM_STYLE_LINE_SPACING_AT_LEAST => LineSpacing::AtLeast(value.number),
                VIEM_STYLE_LINE_SPACING_EXACT => LineSpacing::Exact(value.number),
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::LineSpacing(spacing))
        }
        StyleProperty::ParagraphAlignment => {
            if value.kind != VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let alignment = match value.enum_value {
                VIEM_STYLE_PARAGRAPH_ALIGNMENT_START => ParagraphAlignment::Start,
                VIEM_STYLE_PARAGRAPH_ALIGNMENT_END => ParagraphAlignment::End,
                VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER => ParagraphAlignment::Center,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::ParagraphAlignment(alignment))
        }
    }
}

unsafe fn parse_style_edit_request<O>(
    request: *const ViemStyleEditV1,
    out_outcome: *mut O,
) -> Result<ParsedStyleEdit, ViemStatus> {
    let request = unsafe { read_core_request(request, out_outcome)? };
    if request.struct_size < VIEM_STYLE_EDIT_V1_SIZE
        || request.flags != 0
        || request.reserved != 0
        || request.identity.struct_size < VIEM_STYLE_SHEET_IDENTITY_V1_SIZE
        || request.identity.reserved != 0
    {
        return Err(ViemStatus::InvalidArgument);
    }
    let namespace = parse_style_namespace(request.namespace)?;
    let style = unsafe { composition_utf8(request.style_id, out_outcome)? };
    if style.is_empty() || style.contains('\0') {
        return Err(ViemStatus::InvalidArgument);
    }
    let relationship_value = || unsafe { composition_utf8(request.value.text, out_outcome) };
    let require_empty_value = || {
        if request.value.struct_size < VIEM_STYLE_EDIT_VALUE_V1_SIZE
            || request.value.kind != VIEM_STYLE_VALUE_NONE
            || request.value.reserved != 0
            || request.value.item_count != 0
            || request.value.text.length != 0
        {
            Err(ViemStatus::InvalidStyleValue)
        } else {
            Ok(())
        }
    };
    let edit = match request.operation {
        VIEM_STYLE_EDIT_SET_DECLARATION => {
            let property = parse_style_property(request.property)?;
            StyleDefinitionFieldEdit::SetDeclaration {
                property,
                value: unsafe {
                    parse_style_property_value(property, &request.value, out_outcome)?
                },
            }
        }
        VIEM_STYLE_EDIT_CLEAR_DECLARATION => {
            let property = parse_style_property(request.property)?;
            require_empty_value()?;
            StyleDefinitionFieldEdit::ClearDeclaration(property)
        }
        VIEM_STYLE_EDIT_SET_PARENT
        | VIEM_STYLE_EDIT_SET_NEXT_STYLE
        | VIEM_STYLE_EDIT_SET_DISPLAY_NAME => {
            if request.property != 0
                || request.value.struct_size < VIEM_STYLE_EDIT_VALUE_V1_SIZE
                || request.value.kind != VIEM_STYLE_VALUE_STRING
                || request.value.reserved != 0
                || request.value.item_count != 0
            {
                return Err(ViemStatus::InvalidStyleValue);
            }
            let target = relationship_value()?;
            if target.is_empty() || target.contains('\0') {
                return Err(ViemStatus::InvalidStyleRelationship);
            }
            match request.operation {
                VIEM_STYLE_EDIT_SET_PARENT => {
                    StyleDefinitionFieldEdit::SetParent(Some(StyleId(target)))
                }
                VIEM_STYLE_EDIT_SET_NEXT_STYLE => {
                    StyleDefinitionFieldEdit::SetNextParagraphStyle(Some(StyleId(target)))
                }
                VIEM_STYLE_EDIT_SET_DISPLAY_NAME => {
                    StyleDefinitionFieldEdit::SetDisplayName(target)
                }
                _ => unreachable!("the operation was matched above"),
            }
        }
        VIEM_STYLE_EDIT_CLEAR_PARENT | VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE => {
            if request.property != 0 {
                return Err(ViemStatus::InvalidStyleRelationship);
            }
            require_empty_value()?;
            if request.operation == VIEM_STYLE_EDIT_CLEAR_PARENT {
                StyleDefinitionFieldEdit::SetParent(None)
            } else {
                StyleDefinitionFieldEdit::SetNextParagraphStyle(None)
            }
        }
        _ => return Err(ViemStatus::InvalidArgument),
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
    view: ViemViewId,
    expected_revision: u64,
) -> Result<ViewId, ViemStatus> {
    let view = ViewId(view);
    if core.command_state(view).is_none() {
        return Err(ViemStatus::InvalidView);
    }
    validate_revision(core.document(), expected_revision)?;
    Ok(view)
}

fn dispatch_event(
    core: &mut Core<CTextMeasurementProvider>,
    view: ViemViewId,
    event: CoreEvent,
) -> Result<ViemCoreOutcomeV1, ViemStatus> {
    let view = ViewId(view);
    let outcome = core.handle_with_layout(view, event).map_err(core_status)?;
    summarize_core_outcome(core, view, Some(&outcome))
}

fn dispatch_input_with_effects(
    core: &mut Core<CTextMeasurementProvider>,
    view: ViemViewId,
    input: InputEvent,
    clipboard: ClipboardCommandContext,
) -> Result<(ViemCoreOutcomeV1, Option<OwnedEffectBatch>), ViemStatus> {
    dispatch_event_with_effects(core, view,
        CoreEvent::InputWithClipboard { input, clipboard: clipboard.clone() }, clipboard)
}

fn dispatch_event_with_effects(
    core: &mut Core<CTextMeasurementProvider>,
    view: ViemViewId,
    event: CoreEvent,
    clipboard: ClipboardCommandContext,
) -> Result<(ViemCoreOutcomeV1, Option<OwnedEffectBatch>), ViemStatus> {
    let view_id = ViewId(view);
    let effect_clipboard = clipboard.clone();
    let outcome = core
        .handle_with_layout(view_id, event)
        .map_err(core_status)?;
    let mut summary = summarize_core_outcome(core, view_id, Some(&outcome))?;
    let effects = OwnedEffectBatch::from_command(
        core.document(),
        core.command_state(view_id).ok_or(ViemStatus::InvalidView)?,
        &effect_clipboard,
        outcome.command,
    );
    if effects.is_some() {
        summary.flags |= VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS;
    }
    Ok((summary, effects))
}

fn publish_input_turn(
    handle: ViemCoreHandle,
    view: ViemViewId,
    input: InputEvent,
    clipboard: ClipboardCommandContext,
    reservation: EffectBatchReservation,
) -> Result<(ViemCoreOutcomeV1, ViemEffectBatchHandle), ViemStatus> {
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
pub unsafe extern "C" fn viem_core_create(
    source: *const u8,
    source_length: u64,
    options: *const ViemDocumentOptions,
    out_core: *mut ViemCoreHandle,
    out_revision: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_core.is_null() || out_revision.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        if options.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        let source_length = checked_length(source_length)?;
        if source.is_null() && source_length != 0 {
            return Err(ViemStatus::NullPointer);
        }
        if (out_core as usize) % align_of::<ViemCoreHandle>() != 0
            || (out_revision as usize) % align_of::<u64>() != 0
            || (options as usize) % align_of::<ViemDocumentOptions>() != 0
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<ViemCoreHandle>(),
                out_revision.cast(),
                size_of::<u64>(),
            )
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<ViemCoreHandle>(),
                options.cast(),
                size_of::<ViemDocumentOptions>(),
            )
            || pointer_ranges_overlap(
                out_revision.cast(),
                size_of::<u64>(),
                options.cast(),
                size_of::<ViemDocumentOptions>(),
            )
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<ViemCoreHandle>(),
                source,
                source_length,
            )
            || pointer_ranges_overlap(out_revision.cast(), size_of::<u64>(), source, source_length)
        {
            return Err(ViemStatus::InvalidArgument);
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
/// [`ViemStatus::CoreBusy`] without removing the token. Provider contexts and
/// callbacks must remain valid, and the caller may retry after that operation
/// returns. Only a successful call ends the core and provider lifetimes.
#[no_mangle]
pub extern "C" fn viem_core_destroy(handle: ViemCoreHandle) -> ViemStatus {
    ffi_boundary(|| {
        if handle == 0 {
            return Err(ViemStatus::InvalidHandle);
        }
        let core = {
            let mut registry = core_registry()
                .lock()
                .map_err(|_| ViemStatus::InternalError)?;
            match registry.cores.get(&handle) {
                None => return Err(ViemStatus::InvalidHandle),
                Some(CoreRegistryEntry::Busy) => return Err(ViemStatus::CoreBusy),
                Some(CoreRegistryEntry::Ready(_)) => registry
                    .cores
                    .remove(&handle)
                    .ok_or(ViemStatus::InternalError)?,
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
pub unsafe extern "C" fn viem_core_revision(
    handle: ViemCoreHandle,
    out_revision: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_revision.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        if (out_revision as usize) % align_of::<u64>() != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_document_state(
    handle: ViemCoreHandle,
    out_state: *mut ViemDocumentStateV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_state, 1)?;
        unsafe { out_state.write(ViemDocumentStateV1::default()) };
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
pub unsafe extern "C" fn viem_core_formatted_snapshot_info(
    handle: ViemCoreHandle,
    out_info: *mut ViemFormattedSnapshotInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemFormattedSnapshotInfoV1::default()) };
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
pub unsafe extern "C" fn viem_core_copy_formatted_utf8_range(
    handle: ViemCoreHandle,
    request: *const ViemFormattedUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(output, output_capacity)?;
        let required_region = typed_pointer_region(out_required, 1)?;
        if regions_overlap(output_region, request_region)
            || regions_overlap(output_region, required_region)
            || regions_overlap(required_region, request_region)
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_FORMATTED_UTF8_RANGE_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let start = usize::try_from(request.utf8_start).map_err(|_| ViemStatus::LengthOverflow)?;
        let end = usize::try_from(request.utf8_end).map_err(|_| ViemStatus::LengthOverflow)?;
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
            return Err(ViemStatus::BufferTooSmall);
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
    identity: *const ViemFormattedSnapshotIdentityV1,
    input_offsets: *const u64,
    input_count: u64,
    output_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
}

unsafe fn map_formatted_offsets(
    handle: ViemCoreHandle,
    buffers: FormattedOffsetMappingBuffers,
    mapping: FormattedOffsetMapping,
) -> ViemStatus {
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
            return Err(ViemStatus::InvalidArgument);
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
                        usize::try_from(*offset).map_err(|_| ViemStatus::LengthOverflow)?;
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
            return Err(ViemStatus::BufferTooSmall);
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
pub unsafe extern "C" fn viem_core_map_formatted_utf8_to_utf16(
    handle: ViemCoreHandle,
    identity: *const ViemFormattedSnapshotIdentityV1,
    utf8_offsets: *const u64,
    offset_count: u64,
    utf16_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
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
/// [`viem_core_map_formatted_utf8_to_utf16`].
#[no_mangle]
pub unsafe extern "C" fn viem_core_map_formatted_utf16_to_utf8(
    handle: ViemCoreHandle,
    identity: *const ViemFormattedSnapshotIdentityV1,
    utf16_offsets: *const u64,
    offset_count: u64,
    utf8_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_formatted_point_info(
    handle: ViemCoreHandle,
    identity: *const ViemFormattedSnapshotIdentityV1,
    utf8_offset: u64,
    out_info: *mut ViemFormattedPointInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let identity_region = typed_pointer_region(identity, 1)?;
        let output_region = typed_pointer_region(out_info, 1)?;
        if regions_overlap(identity_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
        }
        let identity = unsafe { read_formatted_snapshot_identity(identity)? };
        unsafe { out_info.write(ViemFormattedPointInfoV1::default()) };
        let utf8_offset = usize::try_from(utf8_offset).map_err(|_| ViemStatus::LengthOverflow)?;
        let info = with_core(handle, |core| {
            validate_formatted_snapshot_identity(identity, core.document())?;
            let snapshot = core.document().hard_line_snapshot();
            let line = snapshot
                .line_at_offset(utf8_offset)
                .map_err(hard_line_query_status)?;
            if !snapshot.is_grapheme_boundary(utf8_offset) {
                return Err(ViemStatus::NotGraphemeBoundary);
            }
            let line_range = line.content_range();
            let grapheme_column = snapshot
                .grapheme_count(line_range.start..utf8_offset)
                .ok_or(ViemStatus::CoreFailure)?;
            Ok(ViemFormattedPointInfoV1 {
                struct_size: VIEM_FORMATTED_POINT_INFO_V1_SIZE,
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
pub unsafe extern "C" fn viem_core_mark_saved(
    handle: ViemCoreHandle,
    request: *const ViemMarkSavedV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(request, 1)?;
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_MARK_SAVED_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_add(
    handle: ViemCoreHandle,
    options: *const ViemViewOptionsV1,
    provider: *const ViemTextMeasurementProviderV1,
    out_view: *mut ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_view.is_null() || out_outcome.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        if (out_view as usize) % align_of::<ViemViewId>() != 0
            || (out_outcome as usize) % align_of::<ViemCoreOutcomeV1>() != 0
            || pointer_ranges_overlap(
                out_view.cast(),
                size_of::<ViemViewId>(),
                out_outcome.cast(),
                size_of::<ViemCoreOutcomeV1>(),
            )
            || (!options.is_null()
                && ((options as usize) % align_of::<ViemViewOptionsV1>() != 0
                    || pointer_ranges_overlap(
                        out_view.cast(),
                        size_of::<ViemViewId>(),
                        options.cast(),
                        size_of::<ViemViewOptionsV1>(),
                    )
                    || pointer_ranges_overlap(
                        out_outcome.cast(),
                        size_of::<ViemCoreOutcomeV1>(),
                        options.cast(),
                        size_of::<ViemViewOptionsV1>(),
                    )))
            || (!provider.is_null()
                && ((provider as usize) % align_of::<ViemTextMeasurementProviderV1>() != 0
                    || pointer_ranges_overlap(
                        out_view.cast(),
                        size_of::<ViemViewId>(),
                        provider.cast(),
                        size_of::<ViemTextMeasurementProviderV1>(),
                    )
                    || pointer_ranges_overlap(
                        out_outcome.cast(),
                        size_of::<ViemCoreOutcomeV1>(),
                        provider.cast(),
                        size_of::<ViemTextMeasurementProviderV1>(),
                    )))
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe {
            out_view.write(0);
            clear_outcome(out_outcome)?;
        }
        if options.is_null() || provider.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        let options = unsafe { options.read() };
        if options.struct_size < VIEM_VIEW_OPTIONS_V1_SIZE
            || !options.width.is_finite()
            || options.width < 0.0
            || !options.height.is_finite()
            || options.height < 0.0
            || [options.padding_top, options.padding_left, options.padding_bottom, options.padding_right]
                .iter().any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let execution_context = parse_execution_context(options.execution_context)?;
        let provider = unsafe { CTextMeasurementProvider::from_ffi(provider)? };
        if provider.render_run_policy.is_none()
            || !execution_context_permits(execution_context, provider.threading)
        {
            return Err(ViemStatus::InvalidProvider);
        }
        let (view, outcome) = with_core_mut(handle, move |core| {
            let mut layout = crate::layout::ViewLayout::new(options.width, options.height);
            layout.set_insets(crate::layout::EdgeInsets {
                top: options.padding_top, left: options.padding_left,
                bottom: options.padding_bottom, right: options.padding_right,
            });
            let view = core
                .try_add_view_with_initial_layout(
                    provider,
                    layout,
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
pub extern "C" fn viem_core_view_remove(handle: ViemCoreHandle, view: ViemViewId) -> ViemStatus {
    ffi_boundary(|| {
        if view == 0 {
            return Err(ViemStatus::InvalidView);
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
pub unsafe extern "C" fn viem_core_view_state(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_style_sheet_info(
    handle: ViemCoreHandle,
    out_info: *mut ViemStyleSheetInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemStyleSheetInfoV1::default()) };
        let info = if handle == 0 { export_code_style_sheet()?.info } else { with_core(handle, |core| Ok(export_style_sheet(core.document())?.info))? };
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
pub unsafe extern "C" fn viem_core_copy_style_sheet(
    handle: ViemCoreHandle,
    expected: *const ViemStyleSheetIdentityV1,
    definitions: *mut ViemStyleDefinitionV1,
    definition_capacity: u64,
    properties: *mut ViemStylePropertyV1,
    property_capacity: u64,
    value_items: *mut ViemStyleValueItemV1,
    value_item_capacity: u64,
    dependencies: *mut ViemStyleDependencyV1,
    dependency_capacity: u64,
    string_bytes: *mut u8,
    string_capacity: u64,
    out_info: *mut ViemStyleSheetInfoV1,
) -> ViemStatus {
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
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_style_sheet_identity(expected)? };
        unsafe { out_info.write(ViemStyleSheetInfoV1::default()) };
        let export = if handle == 0 {
            let sheet = crate::document::code_style::snapshot();
            validate_code_style_identity(expected, &sheet)?;
            export_code_style_snapshot(&sheet)?
        } else { with_core(handle, |core| {
            validate_style_sheet_identity(expected, core.document())?;
            export_style_sheet(core.document())
        })? };
        unsafe { out_info.write(export.info) };
        let fits = definition_capacity >= export.info.definition_count
            && property_capacity >= export.info.property_count
            && value_item_capacity >= export.info.value_item_count
            && dependency_capacity >= export.info.dependency_count
            && string_capacity >= export.info.string_bytes;
        if !fits {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(&export.definitions, definitions);
            copy_output(&export.properties, properties);
            copy_output(&export.value_items, value_items);
            copy_output(&export.dependencies, dependencies);
            copy_output(&export.strings, string_bytes);
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
pub unsafe extern "C" fn viem_core_view_viewport_state(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_state: *mut ViemViewportStateV1,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_state.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        if (out_state as usize) % align_of::<ViemViewportStateV1>() != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_state.write(ViemViewportStateV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_layout_snapshot_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemLayoutSnapshotInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemLayoutSnapshotInfoV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_layout_paint_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemLayoutPaintInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemLayoutPaintInfoV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_copy_layout_paint(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    runs: *mut ViemPaintStyleRunV1,
    run_capacity: u64,
    out_info: *mut ViemLayoutPaintInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(runs, run_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(ViemLayoutPaintInfoV1::default()) };
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
            return Err(ViemStatus::BufferTooSmall);
        };
        debug_assert_eq!(export.info, info);
        unsafe {
            copy_output(&export.runs, runs);
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
pub unsafe extern "C" fn viem_core_view_copy_layout_decorations(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    decorations: *mut ViemLayoutDecorationV1,
    decoration_capacity: u64,
    labels: *mut u8,
    label_capacity: u64,
    out_info: *mut ViemLayoutDecorationsInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(decorations, decoration_capacity)?,
            typed_pointer_region(labels, label_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(ViemLayoutDecorationsInfoV1::default()) };
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
            let info = ViemLayoutDecorationsInfoV1 {
                struct_size: VIEM_LAYOUT_DECORATIONS_INFO_V1_SIZE,
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
                for (row_index, item) in snapshot.decorations_in_paint_order() {
                        values.push(ViemLayoutDecorationV1 {
                            struct_size: VIEM_LAYOUT_DECORATION_V1_SIZE,
                            flags: (if item.render_run.is_some() {
                                VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN
                            } else {
                                0
                            }) | match item.kind {
                                crate::layout::DecorationKind::BlockQuoteBorder | crate::layout::DecorationKind::ThematicBreak => VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER,
                                crate::layout::DecorationKind::BlockBackground => VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND,
                                crate::layout::DecorationKind::BlockBorder => VIEM_LAYOUT_DECORATION_BLOCK_BORDER,
                                _ => 0,
                            },
                            row_index: checked_export_count(row_index)?,
                            label_byte_start: checked_export_count(bytes.len())?,
                            label_byte_length: checked_export_count(item.text.len())?,
                            x: item.x,
                            advance: item.advance,
                            font_size: item.font_size,
                            reserved: 0.0,
                            typographic_bounds: layout_rect_to_ffi(item.typographic_bounds),
                            ink_bounds: layout_rect_to_ffi(item.ink_bounds),
                            render_run: item.render_run.as_ref().map(render_run_to_ffi).unwrap_or_default(),
                            paint: text_paint_to_ffi(&item.paint),
                        });
                        bytes.extend_from_slice(item.text.as_bytes());
                }
            }
            Ok((info, values, bytes))
        })?;
        unsafe { out_info.write(info) };
        if decoration_capacity < info.decoration_count || label_capacity < info.label_bytes {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(&values, decorations);
            copy_output(&bytes, labels);
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
pub unsafe extern "C" fn viem_core_view_copy_layout_snapshot(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    rows: *mut ViemVisualRowV1,
    row_capacity: u64,
    clusters: *mut ViemPositionedClusterV1,
    cluster_capacity: u64,
    carets: *mut ViemPositionedCaretV1,
    caret_capacity: u64,
    out_info: *mut ViemLayoutSnapshotInfoV1,
) -> ViemStatus {
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
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(ViemLayoutSnapshotInfoV1::default()) };
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
            return Err(ViemStatus::BufferTooSmall);
        };
        debug_assert_eq!(export.info, info);
        unsafe {
            copy_output(&export.rows, rows);
            copy_output(&export.clusters, clusters);
            copy_output(&export.carets, carets);
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
pub unsafe extern "C" fn viem_core_view_caret_geometry(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemLayoutCaretRequestV1,
    out_geometry: *mut ViemLayoutCaretGeometryV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(out_geometry, 1)?;
        if regions_overlap(request_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_LAYOUT_CARET_REQUEST_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        if request.identity.struct_size < VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let affinity = parse_layout_affinity(request.affinity)?;
        let text_offset =
            usize::try_from(request.text_offset).map_err(|_| ViemStatus::LengthOverflow)?;
        unsafe { out_geometry.write(ViemLayoutCaretGeometryV1::default()) };
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
            flags |= VIEM_CARET_GEOMETRY_CLUSTER_FALLBACK;
        }
        let geometry = ViemLayoutCaretGeometryV1 {
            struct_size: VIEM_LAYOUT_CARET_GEOMETRY_V1_SIZE,
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
pub unsafe extern "C" fn viem_core_view_layout_hit_test(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemLayoutHitTestRequestV1,
    out_point: *mut ViemLayoutCaretPointV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(out_point, 1)?;
        if regions_overlap(request_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_point.write(ViemLayoutCaretPointV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_presentation(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_presentation: *mut ViemViewPresentationV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_presentation, 1)?;
        unsafe { out_presentation.write(ViemViewPresentationV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_command_line_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemCommandLineInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemCommandLineInfoV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_copy_command_line(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemCommandLineIdentityV1,
    utf8: *mut u8,
    utf8_capacity: u64,
    out_info: *mut ViemCommandLineInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(utf8, utf8_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_command_line_identity(expected)? };
        unsafe { out_info.write(ViemCommandLineInfoV1::default()) };
        let export = with_core(handle, |core| {
            let export = export_command_line(core, ViewId(view))?;
            validate_command_line_identity(expected, export.info.identity)?;
            Ok(export)
        })?;
        unsafe { out_info.write(export.info) };
        if utf8_capacity < export.info.utf8_length {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(&export.bytes, utf8);
        }
        Ok(())
    })
}

/// Read required logical-segment and drawable-rectangle counts for the exact
/// current Visual selection and layout. A non-Visual view returns kind NONE
/// with zero counts and the current layout identity.
/// Linear selections retain their complete logical segments when endpoints
/// are offscreen; rectangles describe only the materialized layout coverage.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_visual_selection_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemVisualSelectionInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemVisualSelectionInfoV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_copy_visual_selection(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemVisualSelectionIdentityV1,
    segments: *mut ViemVisualSelectionSegmentV1,
    segment_capacity: u64,
    rectangles: *mut ViemVisualSelectionRectangleV1,
    rectangle_capacity: u64,
    out_info: *mut ViemVisualSelectionInfoV1,
) -> ViemStatus {
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
        validate_disjoint_regions(&regions)?;
        let expected = unsafe { read_visual_selection_identity(expected)? };
        unsafe { out_info.write(ViemVisualSelectionInfoV1::default()) };
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
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(&export.segments, segments);
            copy_output(&export.rectangles, rectangles);
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
pub unsafe extern "C" fn viem_core_view_semantic_style_presentation(
    handle: ViemCoreHandle,
    view: ViemViewId,
    style: u32,
    out_presentation: *mut ViemSemanticStylePresentationV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_presentation, 1)?;
        unsafe { out_presentation.write(ViemSemanticStylePresentationV1::default()) };
        let style = semantic_style_from_ffi(style)?;
        let (presentation, _) = with_core(handle, |core| {
            export_semantic_style_presentation(core, ViewId(view), style)
        })?;
        unsafe { out_presentation.write(presentation) };
        Ok(())
    })
}

/// Set or clear Markdown Strong/Emphasis on the exact logical selection
/// returned by `viem_core_view_semantic_style_presentation`. Selection or
/// revision changes are rejected, and no layout identity is consulted.
///
/// # Safety
///
/// `request` and `out_outcome` must identify distinct aligned readable and
/// writable values.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_semantic_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetSemanticStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_SET_SEMANTIC_STYLE_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        if request.expected_selection.struct_size < VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE
            || !matches!(
                request.expected_selection.kind,
                VIEM_LOGICAL_SELECTION_KIND_NONE
                    | VIEM_LOGICAL_SELECTION_KIND_CHARACTER
                    | VIEM_LOGICAL_SELECTION_KIND_LINE
            )
        {
            return Err(ViemStatus::InvalidArgument);
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
                .ok_or(ViemStatus::InvalidRange)?;
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
pub unsafe extern "C" fn viem_core_view_use_selection_for_find(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemVisualSelectionIdentityV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(expected_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_reveal_selection(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::RevealSelection)
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Deliver one normalized key with immutable clipboard snapshots and writable
/// capabilities captured by the host for this exact command turn.
///
/// On success `out_effect_batch` receives either zero (no host effects) or an
/// owned immutable batch which the caller must release. Presentation outcomes
/// and host effects are returned independently.
///
/// # Safety
///
/// `input` and `context` (including every nested UTF-8 slice) must remain
/// readable for this call. The two aligned writable outputs must be distinct
/// from all inputs and from each other.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_send_key_with_host_context_v2(
    handle: ViemCoreHandle,
    view: ViemViewId,
    input: *const ViemKeyInputV1,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effect_batch: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(input, 1)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(input_region, outcome_region)
            || regions_overlap(input_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_send_text_with_host_context_v2(
    handle: ViemCoreHandle,
    view: ViemViewId,
    text: *const u8,
    text_length: u64,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effect_batch: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let text_region = typed_pointer_region(text, text_length)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(text_region, outcome_region)
            || regions_overlap(text_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(ViemStatus::InvalidArgument);
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
            .map_err(|_| ViemStatus::InvalidUtf8)?
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
pub unsafe extern "C" fn viem_effect_batch_info(
    handle: ViemEffectBatchHandle,
    out_info: *mut ViemEffectBatchInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemEffectBatchInfoV1::default()) };
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
/// [`ViemStatus::BufferTooSmall`] with complete required counts in `out_info`
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
pub unsafe extern "C" fn viem_effect_batch_copy(
    handle: ViemEffectBatchHandle,
    clipboard_writes: *mut ViemClipboardWriteV1,
    clipboard_write_capacity: u64,
    ex_requests: *mut ViemExFrontendRequestV1,
    ex_request_capacity: u64,
    ex_options: *mut ViemExOptionDisplayV1,
    ex_option_capacity: u64,
    ex_marks: *mut ViemExMarkV1,
    ex_mark_capacity: u64,
    ex_registers: *mut ViemExRegisterV1,
    ex_register_capacity: u64,
    ex_jumps: *mut ViemExJumpV1,
    ex_jump_capacity: u64,
    ex_text_lines: *mut ViemExTextLineV1,
    ex_text_line_capacity: u64,
    file_formats: *mut u32,
    file_format_capacity: u64,
    hard_breaks: *mut u64,
    hard_break_capacity: u64,
    string_bytes: *mut u8,
    string_capacity: u64,
    out_info: *mut ViemEffectBatchInfoV1,
) -> ViemStatus {
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
        validate_disjoint_regions(&regions)?;
        unsafe { out_info.write(ViemEffectBatchInfoV1::default()) };
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
            return Err(ViemStatus::BufferTooSmall);
        }

        unsafe {
            copy_output(&export.clipboard_writes, clipboard_writes);
            copy_output(&export.ex_requests, ex_requests);
            copy_output(&export.ex_options, ex_options);
            copy_output(&export.ex_marks, ex_marks);
            copy_output(&export.ex_registers, ex_registers);
            copy_output(&export.ex_jumps, ex_jumps);
            copy_output(&export.ex_text_lines, ex_text_lines);
            copy_output(&export.file_formats, file_formats);
            copy_output(&export.hard_breaks, hard_breaks);
            copy_output(&export.strings, string_bytes);
        }
        Ok(())
    })
}

/// Release one caller-owned immutable host-effect batch. Handles are never
/// reused; zero, unknown, and already released handles return INVALID_HANDLE.
#[no_mangle]
pub extern "C" fn viem_effect_batch_release(handle: ViemEffectBatchHandle) -> ViemStatus {
    ffi_boundary(|| {
        if handle == 0 {
            return Err(ViemStatus::InvalidHandle);
        }
        let batch = {
            let mut registry = effect_batch_registry()
                .lock()
                .map_err(|_| ViemStatus::InternalError)?;
            if !matches!(
                registry.batches.get(&handle),
                Some(EffectBatchRegistryEntry::Ready(_))
            ) {
                return Err(ViemStatus::InvalidHandle);
            }
            match registry
                .batches
                .remove(&handle)
                .ok_or(ViemStatus::InternalError)?
            {
                EffectBatchRegistryEntry::Ready(batch) => batch,
                EffectBatchRegistryEntry::Reserved => return Err(ViemStatus::InternalError),
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
pub unsafe extern "C" fn viem_core_view_place_cursor(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemPlaceCursorV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_PLACE_CURSOR_V1_SIZE
            || request.flags & !VIEM_PLACE_CURSOR_EXTEND_SELECTION != 0
            || request.reserved != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let text_offset =
            usize::try_from(request.text_offset).map_err(|_| ViemStatus::LengthOverflow)?;
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
                    extend_selection: request.flags & VIEM_PLACE_CURSOR_EXTEND_SELECTION != 0,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Select the full formatted document in an exact source revision.
///
/// # Safety
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_select_all(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SelectAll {
                    document: DocumentId(document),
                    revision: Revision(revision),
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Read one host-global selection option from this buffer.
/// # Safety
/// Output regions must be aligned, writable, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_selection_option(handle: ViemCoreHandle, option_kind: u32, output: *mut u8, capacity: u64, out_required: *mut u64) -> ViemStatus {
    ffi_boundary(|| {
        let keymodel = match option_kind { VIEM_EX_OPTION_KEYMODEL => true, VIEM_EX_OPTION_SELECTMODE | VIEM_EX_OPTION_AUTOSELECT => false, _ => return Err(ViemStatus::InvalidArgument) };
        typed_pointer_region(out_required, 1)?;
        let capacity = checked_length(capacity)?;
        if capacity != 0 { validate_disjoint_regions(&[typed_pointer_region(out_required, 1)?, typed_pointer_region(output, capacity as u64)?])?; }
        let value = with_core(handle, |core| Ok(if option_kind == VIEM_EX_OPTION_AUTOSELECT { if core.selection_options().autoselect { "1" } else { "0" }.to_owned() } else if keymodel { core.selection_options().keymodel.clone() } else { core.selection_options().selectmode.clone() }))?;
        unsafe { out_required.write(value.len() as u64) };
        if capacity < value.len() { return Err(ViemStatus::BufferTooSmall); }
        if !value.is_empty() { unsafe { copy_output(value.as_bytes(), output) }; }
        Ok(())
    })
}
/// Update one option without issuing a modal input command.
/// # Safety
/// `value` must be readable for `length` bytes.
#[no_mangle]
pub unsafe extern "C" fn viem_core_set_selection_option(handle: ViemCoreHandle, option_kind: u32, value: *const u8, length: u64) -> ViemStatus {
    ffi_boundary(|| {
        let keymodel = match option_kind { VIEM_EX_OPTION_KEYMODEL => true, VIEM_EX_OPTION_SELECTMODE | VIEM_EX_OPTION_AUTOSELECT => false, _ => return Err(ViemStatus::InvalidArgument) };
        let value = str::from_utf8(unsafe { input_bytes(value, length)? }).map_err(|_| ViemStatus::InvalidUtf8)?;
        with_core_mut(handle, |core| {
            if option_kind == VIEM_EX_OPTION_AUTOSELECT {
                let enabled = match value { "1" => true, "0" => false, _ => return Err(ViemStatus::InvalidArgument) };
                core.set_autoselect(enabled);
                Ok(())
            } else if core.set_selection_option(keymodel, value) { Ok(()) } else { Err(ViemStatus::InvalidArgument) }
        })
    })
}

/// Apply configured Select/Visual initiation policy without changing the range.
/// # Safety
/// `out_outcome` must be aligned and writable.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_selection_origin(handle: ViemCoreHandle, view: ViemViewId, origin: u32, return_mode: u32, out_outcome: *mut ViemCoreOutcomeV1) -> ViemStatus {
    ffi_boundary(|| {
        let origin = match origin { 1 => crate::command::SelectionOrigin::Mouse, 2 => crate::command::SelectionOrigin::Key, 3 => crate::command::SelectionOrigin::Command, _ => return Err(ViemStatus::InvalidArgument) };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let return_mode = match return_mode { VIEM_MODE_INSERT => Mode::Insert, VIEM_MODE_REPLACE => Mode::Replace, VIEM_MODE_NORMAL => Mode::Normal, _ => return Err(ViemStatus::InvalidArgument) };
            core.set_selection_origin(ViewId(view), origin, return_mode).map_err(core_status)?;
            let mut outcome = summarize_core_outcome(core, ViewId(view), None)?;
            outcome.flags |= VIEM_OUTCOME_MODE_CHANGED;
            Ok(outcome)
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Enter Normal mode and reveal a clamped one-based logical hard line.
/// Pending input is cancelled without executing it or changing source.
///
/// # Safety
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_go_to_line(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    line: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::GoToLine {
                    document: DocumentId(document),
                    revision: Revision(revision),
                    line,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

unsafe fn core_view_navigate_history(
    handle: ViemCoreHandle,
    view: ViemViewId,
    navigation: crate::document::HistoryNavigationRequest,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_view_undo(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_view_redo(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_view_composition_begin(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCompositionBeginV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_COMPOSITION_BEGIN_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_composition_update(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCompositionUpdateV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_COMPOSITION_UPDATE_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_composition_overlay_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemCompositionOverlayInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        let inactive = ViemCompositionOverlayInfoV1 {
            struct_size: VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE,
            identity: ViemCompositionOverlayIdentityV1 {
                struct_size: VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE,
                ..ViemCompositionOverlayIdentityV1::default()
            },
            ..ViemCompositionOverlayInfoV1::default()
        };
        unsafe { out_info.write(inactive) };
        let info = with_core(handle, |core| {
            let view_id = ViewId(view);
            core.command_state(view_id).ok_or(ViemStatus::InvalidView)?;
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
pub unsafe extern "C" fn viem_core_view_copy_composition_utf8_range(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCompositionOverlayUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(output, output_capacity)?;
        let required_region = typed_pointer_region(out_required, 1)?;
        if regions_overlap(output_region, request_region)
            || regions_overlap(output_region, required_region)
            || regions_overlap(required_region, request_region)
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let start = checked_length(request.start)?;
        let end = checked_length(request.end)?;
        let bytes = with_core(handle, |core| {
            let view_id = ViewId(view);
            let overlay = core
                .composition_overlay(view_id)
                .map_err(core_status)?
                .ok_or(ViemStatus::InvalidArgument)?;
            validate_composition_overlay_identity(request.identity, &overlay, view_id)?;
            if start > end || end > overlay.utf8_len() {
                return Err(ViemStatus::InvalidRange);
            }
            overlay
                .text_in_range(start..end)
                .map(String::into_bytes)
                .ok_or(ViemStatus::InvalidUtf8Boundary)
        })?;
        let required = checked_export_count(bytes.len())?;
        unsafe { out_required.write(required) };
        if checked_length(output_capacity)? < bytes.len() {
            return Err(ViemStatus::BufferTooSmall);
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
pub unsafe extern "C" fn viem_core_view_composition_commit(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCompositionCommitV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_COMPOSITION_COMMIT_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_composition_cancel(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCompositionCancelV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_COMPOSITION_CANCEL_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_set_viewport_origin(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemViewportOriginV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_VIEWPORT_ORIGIN_V1_SIZE
            || request.flags & !VIEM_VIEWPORT_ORIGIN_HAS_TOP != 0
            || !request.left.is_finite()
            || (request.flags & VIEM_VIEWPORT_ORIGIN_HAS_TOP != 0 && !request.top.is_finite())
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let top = (request.flags & VIEM_VIEWPORT_ORIGIN_HAS_TOP != 0).then_some(request.top);
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
pub unsafe extern "C" fn viem_core_view_resize(
    handle: ViemCoreHandle,
    view: ViemViewId,
    width: f32,
    height: f32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !width.is_finite() || width < 0.0 || !height.is_finite() || height < 0.0 {
            return Err(ViemStatus::InvalidArgument);
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
pub extern "C" fn viem_core_view_set_padding(
    handle: ViemCoreHandle,
    view: ViemViewId,
    top: f32,
    left: f32,
    bottom: f32,
    right: f32,
) -> ViemStatus {
    ffi_boundary(|| {
        if [top, left, bottom, right]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_adjacent_zoom_scale(
    scale: f32,
    increasing: u32,
    out_scale: *mut f32,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_scale.is_null() || (out_scale as usize) % std::mem::align_of::<f32>() != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_scale.write(0.0) };
        if increasing > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        let next = crate::layout::adjacent_zoom_scale(scale, increasing == 1)
            .map_err(|_| ViemStatus::InvalidArgument)?;
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
pub unsafe extern "C" fn viem_core_view_set_scale(
    handle: ViemCoreHandle,
    view: ViemViewId,
    scale: f32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !crate::layout::valid_zoom_scale(scale) {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_set_wrap(
    handle: ViemCoreHandle,
    view: ViemViewId,
    wrap: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_view_set_linebreak(
    handle: ViemCoreHandle,
    view: ViemViewId,
    linebreak: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !parse_ffi_bool(linebreak)? {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_set_file_format(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetFileFormatV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_SET_FILE_FORMAT_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
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

/// Query an exact list-action target without requiring current layout.
///
/// # Safety
/// The output must identify an aligned writable selection identity.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_list_selection(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_selection: *mut ViemLogicalSelectionIdentityV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_selection, 1)?;
        unsafe { out_selection.write(ViemLogicalSelectionIdentityV1::default()) };
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
pub unsafe extern "C" fn viem_core_view_list_indent_capabilities(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLogicalSelectionIdentityV1,
    out_flags: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_flags, 1)?;
        if regions_overlap(input_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
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
            Ok(u32::from(indent) * VIEM_LIST_CAN_INDENT
                | u32::from(unindent) * VIEM_LIST_CAN_UNINDENT)
        })?;
        unsafe { out_flags.write(flags) };
        Ok(())
    })
}

/// Indent/unindent complete selected list items by exactly one level.
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_indent_list(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemListIndentV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_LIST_INDENT_V1_SIZE || request.unindent > 1 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_set_list_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetListStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_SET_LIST_STYLE_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        let style = match request.style {
            VIEM_LIST_STYLE_NONE => None,
            VIEM_LIST_STYLE_BULLET => Some(crate::document::ListStyle::Bullet),
            VIEM_LIST_STYLE_NUMBERED => Some(crate::document::ListStyle::Numbered),
            _ => return Err(ViemStatus::InvalidArgument),
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
pub unsafe extern "C" fn viem_core_view_set_paragraph_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetParagraphStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_SET_PARAGRAPH_STYLE_V1_SIZE || request.level > 6 {
            return Err(ViemStatus::InvalidArgument);
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
/// updates the pending typing style. Paragraph assignment targets the current
/// paragraph; Block quote at the end of ordinary prose creates a blank quote.
///
/// # Safety
/// Request, its UTF-8 slice, and outcome must be valid and not overlap output.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_assign_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemAssignStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_ASSIGN_STYLE_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        let namespace = parse_style_namespace(request.namespace)?;
        let style = unsafe { composition_utf8(request.style_id, out_outcome)? };
        if (style.is_empty() && namespace != StyleNamespace::Character) || style.contains('\0') {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_edit_direct_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemDirectStyleEditV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_DIRECT_STYLE_EDIT_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        let property = parse_style_property(request.property)?;
        if CANVAS_STYLE_PROPERTIES.contains(&property) {
            return Err(ViemStatus::UnsupportedOperation);
        }
        let value = match request.operation {
            VIEM_STYLE_EDIT_SET_DECLARATION => {
                Some(unsafe { parse_direct_style_property_value(property, &request.value, out_outcome)? })
            }
            VIEM_STYLE_EDIT_CLEAR_DECLARATION
                if request.value.struct_size >= VIEM_STYLE_EDIT_VALUE_V1_SIZE
                    && request.value.kind == VIEM_STYLE_VALUE_NONE
                    && request.value.reserved == 0
                    && request.value.item_count == 0
                    && request.value.text.length == 0 =>
            {
                None
            }
            _ => return Err(ViemStatus::InvalidArgument),
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
pub unsafe extern "C" fn viem_core_view_edit_direct_character_batch(
    handle: ViemCoreHandle,
    view: ViemViewId,
    requests: *const ViemDirectStyleEditV1,
    count: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        if !(1..=32).contains(&count) {
            return Err(ViemStatus::InvalidArgument);
        }
        let input = typed_pointer_region(requests, count)?;
        let output = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(input, output) {
            return Err(ViemStatus::InvalidArgument);
        }
        let requests = unsafe { std::slice::from_raw_parts(requests, count as usize) };
        let expected_selection = requests[0].expected_selection;
        let mut values = Vec::with_capacity(count as usize);
        let mut seen = std::collections::BTreeSet::new();
        for request in requests {
            if request.struct_size < VIEM_DIRECT_STYLE_EDIT_V1_SIZE
                || request.reserved != 0
                || request.operation != VIEM_STYLE_EDIT_SET_DECLARATION
            {
                return Err(ViemStatus::InvalidArgument);
            }
            validate_logical_selection_identity(request.expected_selection, expected_selection)?;
            let property = parse_style_property(request.property)?;
            if !crate::document::is_character_property(property) || !seen.insert(property) {
                return Err(ViemStatus::InvalidArgument);
            }
            let value =
                unsafe { parse_direct_style_property_value(property, &request.value, out_outcome)? };
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
pub unsafe extern "C" fn viem_core_view_decoration_state(
    handle: ViemCoreHandle,
    view: ViemViewId,
    property: u32,
    out_state: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_state, 1)?;
        let strike = match parse_style_property(property)? {
            StyleProperty::CharacterUnderline => false,
            StyleProperty::CharacterStrikethrough => true,
            _ => return Err(ViemStatus::InvalidArgument),
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
pub struct ViemTypographyInfoV1 {
    pub struct_size: u32,
    /// bit0 bold, bit1 mixed, bit2 default foreground, bit3 mixed script.
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub font_family_bytes: u64,
    pub feature_count: u64,
    pub size: f32,
    pub weight: u32,
    pub base_weight: u32,
    pub slant: u32,
    pub foreground: ViemRgbaV1,
    pub script_position: u32,
    pub has_background: u32,
    pub background: ViemRgbaV1,
}

/// # Safety
/// Outputs must be valid for their capacities and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_typography_export(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected_revision: u64,
    out_info: *mut ViemTypographyInfoV1,
    out_family: *mut u8,
    family_capacity: u64,
    out_features: *mut ViemOpenTypeFeatureV1,
    feature_capacity: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        let family_region = typed_pointer_region(out_family, family_capacity)?;
        let feature_region = typed_pointer_region(out_features, feature_capacity)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        if regions_overlap(info_region, family_region)
            || regions_overlap(info_region, feature_region)
            || regions_overlap(family_region, feature_region)
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let (info, family, features) = with_core(handle, |core| {
            if core.document().revision().0 != expected_revision {
                return Err(ViemStatus::StaleRevision);
            }
            let (style, mixed, script_mixed) = core
                .selected_typography_details(ViewId(view))
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
                .map(|(tag, value)| ViemOpenTypeFeatureV1 {
                    tag: tag.as_bytes().try_into().unwrap_or(*b"    "),
                    value: *value,
                })
                .collect::<Vec<_>>();
            let info = ViemTypographyInfoV1 {
                struct_size: size_of::<ViemTypographyInfoV1>() as u32,
                flags: u32::from(style.bold)
                    | (u32::from(mixed) << 1)
                    | (u32::from(style.foreground_is_default) << 2)
                    | (u32::from(script_mixed) << 3),
                document_id: core.document().id().0,
                document_revision: expected_revision,
                font_family_bytes: family.len() as u64,
                feature_count: features.len() as u64,
                size: style.size,
                weight: u32::from(style.weight),
                base_weight: u32::from(style.base_weight),
                slant: match style.slant {
                    FontSlant::Upright => VIEM_FONT_SLANT_UPRIGHT,
                    FontSlant::Italic => VIEM_FONT_SLANT_ITALIC,
                    FontSlant::Oblique => VIEM_FONT_SLANT_OBLIQUE,
                },
                foreground: ViemRgbaV1 {
                    red: style.foreground.red,
                    green: style.foreground.green,
                    blue: style.foreground.blue,
                    alpha: style.foreground.alpha,
                },
                script_position: style.script_position as u32,
                has_background: u32::from(style.background.is_some()),
                background: style.background.map(color_to_ffi).unwrap_or_default(),
            };
            Ok((info, family, features))
        })?;
        unsafe {
            out_info.write(info);
        }
        if family_capacity < info.font_family_bytes || feature_capacity < info.feature_count {
            return Err(ViemStatus::BufferTooSmall);
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
pub unsafe extern "C" fn viem_core_view_create_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemCreateStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_CREATE_STYLE_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
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
            return Err(ViemStatus::InvalidArgument);
        }
        if namespace == StyleNamespace::Character && !next.is_empty() {
            return Err(ViemStatus::InvalidStyleRelationship);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_style_sheet_identity(request.identity, core.document())?;
            let sheet = core.document().projection().style_sheet();
            let parent = if parent.is_empty() {
                (namespace == StyleNamespace::Block).then(|| sheet.base_paragraph.clone())
            } else { Some(StyleId(parent)) };
            let role = parent.as_ref().and_then(|id| sheet.block_style(id)).map_or(BlockRole::Paragraph, |s| if s.role.is_container() { s.role } else { BlockRole::Paragraph });
            let metadata = crate::document::StyleDefinitionMetadata {
                display_name: name,
                origin: StyleDefinitionOrigin::SourceBacked,
            };
            let edit = match namespace {
                StyleNamespace::Block => crate::document::StyleDefinitionEdit::InsertBlock {
                    style: crate::document::BlockStyle {
                        id: StyleId(id),
                        based_on: parent,
                        next_paragraph_style: (!next.is_empty()).then_some(StyleId(next)),
                        role,
                        character: CharacterProperties::default(),
                        block: BlockProperties::default(),
                    },
                    metadata,
                },
                StyleNamespace::Character => {
                    crate::document::StyleDefinitionEdit::InsertCharacter {
                        style: crate::document::CharacterStyle {
                            id: StyleId(id),
                            based_on: parent,
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
pub unsafe extern "C" fn viem_core_view_delete_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemDeleteStyleV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_DELETE_STYLE_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        let namespace = parse_style_namespace(request.namespace)?;
        let id = unsafe { composition_utf8(request.style_id, out_outcome)? };
        if id.is_empty() || id.contains('\0') {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_begin_style_edit_group(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemStyleSheetIdentityV1,
    out_group: *mut ViemStyleEditGroupV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_group, 1)?;
        if regions_overlap(expected_region, output_region) {
            return Err(ViemStatus::InvalidArgument);
        }
        let expected = unsafe { read_style_sheet_identity(expected)? };
        unsafe { out_group.write(ViemStyleEditGroupV1::default()) };
        let group = with_core_mut(handle, |core| {
            core.begin_style_edit_group(
                ViewId(view),
                DocumentId(expected.document_id),
                Revision(expected.document_revision),
                StyleSheetRevision(expected.style_sheet_revision),
            )
            .map_err(core_status)
        })?;
        unsafe { out_group.write(ViemStyleEditGroupV1::from_core(group)) };
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
pub unsafe extern "C" fn viem_core_view_edit_style(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemStyleEditV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_view_edit_style_in_group(
    handle: ViemCoreHandle,
    view: ViemViewId,
    group: *const ViemStyleEditGroupV1,
    request: *const ViemStyleEditV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(group, 1)?,
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
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
/// `VIEM_STATUS_INVALID_STYLE_EDIT_GROUP`.
///
/// # Safety
///
/// `group` must identify one aligned readable v1 value.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_end_style_edit_group(
    handle: ViemCoreHandle,
    view: ViemViewId,
    group: *const ViemStyleEditGroupV1,
) -> ViemStatus {
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
    handle: ViemCoreHandle,
    expected_revision: u64,
    source: bool,
) -> Result<Vec<u8>, ViemStatus> {
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
    handle: ViemCoreHandle,
    expected_revision: u64,
    source: bool,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_required.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        let capacity = checked_length(output_capacity)?;
        if output.is_null() && capacity != 0 {
            return Err(ViemStatus::NullPointer);
        }
        if (out_required as usize) % align_of::<u64>() != 0
            || pointer_ranges_overlap(output, capacity, out_required.cast(), size_of::<u64>())
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let bytes = core_snapshot_bytes(handle, expected_revision, source)?;
        let required = u64::try_from(bytes.len()).map_err(|_| ViemStatus::LengthOverflow)?;
        unsafe { out_required.write(required) };
        if capacity < bytes.len() {
            return Err(ViemStatus::BufferTooSmall);
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if output.is_null() {
            return Err(ViemStatus::NullPointer);
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
pub unsafe extern "C" fn viem_core_copy_source_bytes(
    handle: ViemCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
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
pub unsafe extern "C" fn viem_core_copy_formatted_utf8(
    handle: ViemCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
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
    mod prelayout_tests;
    mod html_export_tests;
    mod clipboard_import_tests;
    mod theme_tests;
    use super::{
        checkout_core, viem_core_copy_formatted_utf8_range,
        viem_core_copy_style_sheet, viem_core_destroy, viem_core_formatted_point_info,
        viem_core_formatted_snapshot_info, viem_core_map_formatted_utf16_to_utf8,
        viem_core_map_formatted_utf8_to_utf16, viem_core_style_sheet_info,
        viem_core_view_copy_layout_paint, viem_core_view_layout_paint_info,
        export_style_sheet, ffi_boundary, register_core,
        summarize_document_state, CTextMeasurementProvider, ViemClusterCaretStopV1,
        ViemCoreOutcomeV1, ViemFormattedPointInfoV1, ViemFormattedSnapshotInfoV1,
        ViemFormattedUtf8RangeV1, ViemLayoutPaintInfoV1, ViemPaintStyleRunV1,
        ViemRenderRunHandleV1, ViemShapeRequestV1, ViemShapeResponseV1, ViemShapedBoundsV1,
        ViemShapedClusterV1, ViemStatus, ViemStyleDefinitionV1, ViemStyleDependencyV1,
        ViemStyleEditV1, ViemStylePropertyV1, ViemStyleSheetInfoV1, ViemStyleStringRefV1,
        ViemStyleValueItemV1, ViemTextMetricsV1, ViemUtf8Slice, MarshalledRequest,
        VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, VIEM_BOUNDARY_AFFINITY_UPSTREAM,
        VIEM_HISTORY_ACTION_CATEGORY_MIXED, VIEM_LAYOUT_PAINT_INFO_V1_SIZE,
        VIEM_PAINT_STYLE_RUN_V1_SIZE, VIEM_SHAPED_CLUSTER_V1_SIZE,
        VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA,
        VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT, VIEM_TEXT_PAINT_HAS_BACKGROUND,
        VIEM_TEXT_PAINT_STRIKETHROUGH, VIEM_TEXT_PAINT_UNDERLINE, VIEM_TEXT_PAINT_V1_SIZE,
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
            VIEM_HISTORY_ACTION_CATEGORY_MIXED
        );
        document.try_undo().unwrap();
        assert_eq!(
            summarize_document_state(&document).redo_action_category,
            VIEM_HISTORY_ACTION_CATEGORY_MIXED
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

        fn referenced_text(arena: &[u8], reference: super::ViemEffectBytesRefV1) -> &str {
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

        let mut info = super::ViemEffectBatchInfoV1::default();
        assert_eq!(
            unsafe { super::viem_effect_batch_info(handle, &mut info) },
            ViemStatus::Ok
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
            super::VIEM_EFFECT_BATCH_HAS_EX_OUTCOME
                | super::VIEM_EFFECT_BATCH_EX_DOCUMENT_CHANGED
                | super::VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION
        );

        let mut writes = vec![
            super::ViemClipboardWriteV1 {
                target: u32::MAX,
                ..super::ViemClipboardWriteV1::default()
            };
            info.clipboard_write_count as usize
        ];
        let mut requests = vec![
            super::ViemExFrontendRequestV1 {
                kind: u32::MAX,
                ..super::ViemExFrontendRequestV1::default()
            };
            info.ex_request_count as usize
        ];
        let mut options = vec![
            super::ViemExOptionDisplayV1 {
                name: u32::MAX,
                ..super::ViemExOptionDisplayV1::default()
            };
            info.ex_option_count as usize
        ];
        let mut marks = vec![
            super::ViemExMarkV1 {
                name: u32::MAX,
                ..super::ViemExMarkV1::default()
            };
            info.ex_mark_count as usize
        ];
        let mut registers = vec![
            super::ViemExRegisterV1 {
                name: u32::MAX,
                ..super::ViemExRegisterV1::default()
            };
            info.ex_register_count as usize
        ];
        let mut jumps = vec![
            super::ViemExJumpV1 {
                flags: u32::MAX,
                ..super::ViemExJumpV1::default()
            };
            info.ex_jump_count as usize
        ];
        let mut text_lines = vec![
            super::ViemExTextLineV1 {
                hard_line_index: u64::MAX,
                ..super::ViemExTextLineV1::default()
            };
            info.ex_text_line_count as usize
        ];
        let mut file_formats = vec![u32::MAX; info.file_format_count as usize];
        let mut hard_breaks = vec![u64::MAX; info.hard_break_count as usize];
        let mut strings = vec![0xa5; info.string_bytes as usize];
        let mut copied_info = super::ViemEffectBatchInfoV1::default();
        assert_eq!(
            unsafe {
                super::viem_effect_batch_copy(
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
            ViemStatus::BufferTooSmall
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
                super::viem_effect_batch_copy(
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
            ViemStatus::Ok
        );
        assert_eq!(copied_info, info);
        assert_eq!(writes[0].target, super::VIEM_CLIPBOARD_TARGET_CLIPBOARD);
        assert_eq!(writes[0].flags, 0);
        assert_eq!(writes[0].register_kind, super::VIEM_REGISTER_KIND_NONE);
        assert_eq!(referenced_text(&strings, writes[0].plain_text), "plain 👋");
        assert_eq!(writes[1].target, super::VIEM_CLIPBOARD_TARGET_PRIMARY);
        assert_eq!(
            writes[1].flags,
            super::VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER
        );
        assert_eq!(writes[1].register_kind, super::VIEM_REGISTER_KIND_LINE);
        assert_eq!(referenced_text(&strings, writes[1].plain_text), "α\nβ");
        assert_eq!(hard_breaks, vec![2, 3]);

        assert_eq!(
            requests
                .iter()
                .map(|request| request.kind)
                .collect::<Vec<_>>(),
            (super::VIEM_EX_FRONTEND_EDIT..=super::VIEM_EX_FRONTEND_NORMAL).collect::<Vec<_>>()
        );
        assert!(requests
            .iter()
            .all(|request| { request.document_id == 0xdecaf && request.document_revision == 42 }));
        assert_eq!(
            requests[0].flags,
            super::VIEM_EX_FRONTEND_FORCE | super::VIEM_EX_FRONTEND_HAS_PATH
        );
        assert_eq!(referenced_text(&strings, requests[0].text), "文章/é.md");
        assert_eq!(
            requests[2].flags,
            super::VIEM_EX_FRONTEND_FORCE
                | super::VIEM_EX_FRONTEND_HAS_PATH
                | super::VIEM_EX_FRONTEND_HAS_RANGE
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
                super::VIEM_EX_OPTION_WRAP,
                super::VIEM_EX_OPTION_IGNORECASE,
                super::VIEM_EX_OPTION_FILE_FORMAT,
                super::VIEM_EX_OPTION_FILE_FORMATS,
            ]
        );
        assert_eq!(options[0].scalar_value, 1);
        assert_eq!(options[1].scalar_value, 0);
        assert_eq!(options[2].scalar_value, super::VIEM_FILE_FORMAT_DOS);
        assert_eq!(
            (options[3].first_file_format, options[3].file_format_count),
            (0, 3)
        );
        assert_eq!(
            file_formats,
            vec![
                super::VIEM_FILE_FORMAT_UNIX,
                super::VIEM_FILE_FORMAT_DOS,
                super::VIEM_FILE_FORMAT_MAC,
            ]
        );
        assert_eq!(
            requests[13].flags,
            super::VIEM_EX_FRONTEND_HAS_RANGE
                | super::VIEM_EX_FRONTEND_NUMBER
                | super::VIEM_EX_FRONTEND_LIST
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
            super::VIEM_EX_FRONTEND_HAS_RANGE | super::VIEM_EX_FRONTEND_LITERAL
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
            super::VIEM_REGISTER_KIND_CHARACTER
        );
        assert_eq!(referenced_text(&strings, registers[0].text), "一\n二");
        assert_eq!(
            (registers[0].first_hard_break, registers[0].hard_break_count),
            (1, 1)
        );
        assert_eq!(jumps[0].flags, super::VIEM_EX_JUMP_CURRENT);
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
            super::VIEM_EFFECT_BATCH_HAS_EX_OUTCOME
                | super::VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION
                | super::VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY
        );

        assert_eq!(super::viem_effect_batch_release(handle), ViemStatus::Ok);
        info.batch_handle = u64::MAX;
        assert_eq!(
            unsafe { super::viem_effect_batch_info(handle, &mut info) },
            ViemStatus::InvalidHandle
        );
        assert_eq!(info, super::ViemEffectBatchInfoV1::default());
        assert_eq!(
            super::viem_effect_batch_release(handle),
            ViemStatus::InvalidHandle
        );
        assert_eq!(
            super::viem_effect_batch_release(0),
            ViemStatus::InvalidHandle
        );
    }

    #[test]
    fn formatted_projection_ffi_is_exact_unicode_safe_and_all_or_none() {
        let text = "Aé👩‍💻e\u{301}\nlast";
        let handle = register_core(Core::new(Document::new(text))).unwrap();

        let mut info = ViemFormattedSnapshotInfoV1::default();
        assert_eq!(
            unsafe { viem_core_formatted_snapshot_info(handle, &mut info) },
            ViemStatus::Ok
        );
        assert_eq!(info.utf8_length, text.len() as u64);
        assert_eq!(info.utf16_length, text.encode_utf16().count() as u64);
        assert_eq!(info.hard_line_count, 2);

        let range = ViemFormattedUtf8RangeV1 {
            struct_size: super::VIEM_FORMATTED_UTF8_RANGE_V1_SIZE,
            reserved: 0,
            identity: info.identity,
            utf8_start: 3,
            utf8_end: 17,
        };
        let mut required = u64::MAX;
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(required, 14);

        let mut short = vec![0xcc; required as usize - 1];
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    short.as_mut_ptr(),
                    short.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert!(short.iter().all(|byte| *byte == 0xcc));

        let mut copied = vec![0; required as usize];
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    copied.as_mut_ptr(),
                    copied.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(std::str::from_utf8(&copied).unwrap(), "👩‍💻e\u{301}");

        let utf8 = [0, 1, 3, 7, 10, 14, 15, 17, 18, 22];
        let expected_utf16 = [0, 1, 2, 4, 5, 7, 8, 9, 10, 14];
        required = 0;
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(required, utf8.len() as u64);

        let mut short_offsets = vec![u64::MAX; utf8.len() - 1];
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    short_offsets.as_mut_ptr(),
                    short_offsets.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert!(short_offsets.iter().all(|offset| *offset == u64::MAX));

        let mut utf16 = vec![u64::MAX; utf8.len()];
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    utf16.as_mut_ptr(),
                    utf16.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(utf16, expected_utf16);

        let mut round_trip = vec![u64::MAX; utf8.len()];
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf16_to_utf8(
                    handle,
                    &info.identity,
                    utf16.as_ptr(),
                    utf16.len() as u64,
                    round_trip.as_mut_ptr(),
                    round_trip.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(round_trip, utf8);

        let bad_utf8 = [0, 2];
        let mut untouched = [77, 88];
        required = 91;
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    bad_utf8.as_ptr(),
                    bad_utf8.len() as u64,
                    untouched.as_mut_ptr(),
                    untouched.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::InvalidUtf8Boundary
        );
        assert_eq!(untouched, [77, 88]);
        assert_eq!(required, 0);

        let bad_utf16 = [0, 3];
        required = 91;
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf16_to_utf8(
                    handle,
                    &info.identity,
                    bad_utf16.as_ptr(),
                    bad_utf16.len() as u64,
                    untouched.as_mut_ptr(),
                    untouched.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::InvalidUtf16Boundary
        );
        assert_eq!(untouched, [77, 88]);
        assert_eq!(required, 0);

        let mut point = ViemFormattedPointInfoV1::default();
        assert_eq!(
            unsafe { viem_core_formatted_point_info(handle, &info.identity, 17, &mut point) },
            ViemStatus::Ok
        );
        assert_eq!(point.identity, info.identity);
        assert_eq!(point.utf16_offset, 9);
        assert_eq!(point.hard_line_index, 0);
        assert_eq!((point.hard_line_start, point.hard_line_end), (0, 17));
        assert_eq!(point.grapheme_column, 4);

        assert_eq!(
            unsafe { viem_core_formatted_point_info(handle, &info.identity, 15, &mut point) },
            ViemStatus::NotGraphemeBoundary
        );
        assert_eq!(point, ViemFormattedPointInfoV1::default());

        let invalid_range = ViemFormattedUtf8RangeV1 {
            utf8_start: 2,
            utf8_end: 3,
            ..range
        };
        let mut range_output = [0xa5; 4];
        required = 91;
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &invalid_range,
                    range_output.as_mut_ptr(),
                    range_output.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::InvalidUtf8Boundary
        );
        assert_eq!(range_output, [0xa5; 4]);
        assert_eq!(required, 0);

        let mut stale = info.identity;
        stale.document_revision += 1;
        let stale_range = ViemFormattedUtf8RangeV1 {
            identity: stale,
            ..range
        };
        required = 91;
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &stale_range,
                    range_output.as_mut_ptr(),
                    range_output.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::StaleRevision
        );
        assert_eq!(range_output, [0xa5; 4]);
        assert_eq!(required, 0);
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn bounded_projection_ffi_does_not_materialize_compatibility_text() {
        let mut document = Document::new("x\n".repeat(50_000));
        document.insert(50_000, "Z").unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
        let handle = register_core(Core::new(document)).unwrap();

        let mut info = ViemFormattedSnapshotInfoV1::default();
        assert_eq!(
            unsafe { viem_core_formatted_snapshot_info(handle, &mut info) },
            ViemStatus::Ok
        );
        let request = ViemFormattedUtf8RangeV1 {
            struct_size: super::VIEM_FORMATTED_UTF8_RANGE_V1_SIZE,
            identity: info.identity,
            utf8_start: 49_996,
            utf8_end: 50_008,
            ..ViemFormattedUtf8RangeV1::default()
        };
        let mut output = [0_u8; 12];
        let mut required = 0;
        assert_eq!(
            unsafe {
                viem_core_copy_formatted_utf8_range(
                    handle,
                    &request,
                    output.as_mut_ptr(),
                    output.len() as u64,
                    &mut required,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(required, 12);
        let input = [50_000];
        let mut mapped = [0];
        assert_eq!(
            unsafe {
                viem_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    input.as_ptr(),
                    1,
                    mapped.as_mut_ptr(),
                    1,
                    &mut required,
                )
            },
            ViemStatus::Ok
        );
        let mut point = ViemFormattedPointInfoV1::default();
        assert_eq!(
            unsafe { viem_core_formatted_point_info(handle, &info.identity, 50_000, &mut point) },
            ViemStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert!(!lease
                .core()
                .document()
                .projection()
                .compatibility_text_is_materialized());
        }
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    fn style_arena_text(arena: &[u8], range: ViemStyleStringRefV1) -> &str {
        let start = usize::try_from(range.offset).unwrap();
        let length = usize::try_from(range.length).unwrap();
        std::str::from_utf8(&arena[start..start + length]).unwrap()
    }

    #[test]
    fn style_defaults_diagnostics_preserve_valid_settings_and_release_the_core_lease() {
        use super::*;
        struct Diagnostics {
            handle: ViemCoreHandle,
            messages: Vec<String>,
            read_statuses: Vec<ViemStatus>,
        }
        unsafe extern "C" fn collect(context: *mut c_void, message: *const u8, length: u64) {
            let diagnostics = unsafe { &mut *context.cast::<Diagnostics>() };
            let bytes = unsafe { std::slice::from_raw_parts(message, length as usize) };
            diagnostics.messages.push(String::from_utf8_lossy(bytes).into_owned());
            let mut state = ViemDocumentStateV1::default();
            diagnostics.read_statuses.push(unsafe {
                viem_core_document_state(diagnostics.handle, &mut state)
            });
        }
        let source = "> Quote\n\n```\ncode\n```";
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let handle = register_core(Core::new(document)).unwrap();
        let mut diagnostics = Diagnostics { handle, messages: Vec::new(), read_statuses: Vec::new() };
        let settings = br#"{"version":1,"block_styles":[
          {"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","based_on":null,
           "next_paragraph_style":null,"block":{},"character":{"size":23}},
          {"id":"Block quote","name":"Block quote","role":"Paragraph","based_on":"Paragraph",
           "next_paragraph_style":"Block quote","block":{},"character":{}}
        ]}"#;
        assert_eq!(unsafe {
            viem_core_initialize_style_defaults(handle, 0, settings.as_ptr(), settings.len() as u64,
                Some(collect), (&mut diagnostics as *mut Diagnostics).cast())
        }, ViemStatus::Ok);
        assert!(diagnostics.messages.iter().any(|message| message.contains("Block quote")));
        assert!(diagnostics.read_statuses.iter().all(|status| *status == ViemStatus::Ok));
        let configured = {
            let lease = checkout_core(handle).unwrap();
            let document = lease.core().document();
            let sheet = document.projection().style_sheet();
            assert_eq!(sheet.block_style(&"Paragraph".into()).unwrap().character.size,
                Some(crate::document::FontSize::Points(23.)));
            assert_eq!(sheet.block_style(&"Block quote".into()).unwrap().role,
                crate::document::BlockRole::Quote);
            assert_eq!(document.source_bytes(), source.as_bytes());
            document.export_style_defaults().unwrap()
        };
        diagnostics.messages.clear();
        let invalid = br#"{"version":99}"#;
        assert_eq!(unsafe {
            viem_core_initialize_style_defaults(handle, 0, invalid.as_ptr(), invalid.len() as u64,
                Some(collect), (&mut diagnostics as *mut Diagnostics).cast())
        }, ViemStatus::InvalidArgument);
        assert!(diagnostics.messages.iter().any(|message| message.contains("99")));
        assert!(diagnostics.read_statuses.iter().all(|status| *status == ViemStatus::Ok));
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().export_style_defaults().unwrap(), configured);
        }
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn style_sheet_export_exposes_builtin_default_deltas_after_loading_configuration() {
        use super::*;
        use crate::document::Revision;
        for (format, source) in [
            (Format::PlainText, "Text"),
            (Format::Markdown, "# Heading"),
            (Format::MarkdownSource, "# Heading"),

            (Format::Rtf, r"{\rtf1 Text}"),
        ] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            document.initialize_style_defaults(br#"{"version":1}"#).unwrap();
            let export = export_style_sheet(&document).unwrap();
            let heading = export.definitions.iter().find(|definition|
                style_arena_text(&export.strings, definition.stable_id) == "Heading1").unwrap();
            assert_eq!(style_arena_text(&export.strings, heading.parent_id), "Paragraph");
            let properties = &export.properties[heading.first_property as usize
                ..(heading.first_property + heading.property_count) as usize];
            for (key, expected) in [
                (VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 24.),
                (VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP, 10.),
                (VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM, 5.),
            ] {
                let property = properties.iter().find(|property| property.property == key).unwrap();
                assert_ne!(property.flags & VIEM_STYLE_PROPERTY_DECLARED, 0, "{format:?} property {key}");
                assert_eq!(property.declared.number, expected, "{format:?}");
                assert_eq!(property.effective.number, expected, "{format:?}");
                assert_eq!(style_arena_text(&export.strings, property.contributor_style_id), "Heading1");
            }
            let weight = properties.iter().find(|property| property.property == VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).unwrap();
            assert_ne!(weight.flags & VIEM_STYLE_PROPERTY_DECLARED, 0, "{format:?}");
            assert_eq!(weight.declared.enum_value, 700, "{format:?}");
            let family = properties.iter().find(|property| property.property == VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES).unwrap();
            assert_eq!(family.flags & VIEM_STYLE_PROPERTY_DECLARED, 0, "parent values stay inherited in {format:?}");
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert_eq!(document.revision(), Revision(0));
        }
    }

    #[test]
    fn style_sheet_export_is_exact_typed_and_two_pass_without_partial_writes() {
        let handle = register_core(Core::new(Document::new("plain"))).unwrap();
        let mut info = ViemStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { viem_core_style_sheet_info(handle, &mut info) },
            ViemStatus::Ok
        );
        assert_eq!(info.definition_count, 22);
        assert_eq!(info.property_count, 768);
        assert_ne!(info.string_bytes, 0);

        let mut count_info = ViemStyleSheetInfoV1::default();
        assert_eq!(
            unsafe {
                viem_core_copy_style_sheet(
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
            ViemStatus::BufferTooSmall
        );
        assert_eq!(count_info, info);

        let mut definitions =
            vec![ViemStyleDefinitionV1::default(); info.definition_count as usize];
        let mut properties = vec![ViemStylePropertyV1::default(); info.property_count as usize];
        let mut value_items = vec![ViemStyleValueItemV1::default(); info.value_item_count as usize];
        let mut dependencies =
            vec![ViemStyleDependencyV1::default(); info.dependency_count as usize];
        let mut strings = vec![0_u8; info.string_bytes as usize];
        let mut copied = ViemStyleSheetInfoV1::default();
        assert_eq!(
            unsafe {
                viem_core_copy_style_sheet(
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
            ViemStatus::Ok
        );
        assert_eq!(copied, info);

        let document = definitions
            .iter()
            .find(|definition| style_arena_text(&strings, definition.stable_id) == "Paragraph")
            .unwrap();
        assert_eq!(
            style_arena_text(&strings, document.display_name),
            "Base Paragraph"
        );
        assert_ne!(
            style_arena_text(&strings, document.stable_id),
            style_arena_text(&strings, document.display_name)
        );
        assert_eq!(document.namespace, super::VIEM_STYLE_NAMESPACE_BLOCK);
        assert_eq!(document.role, super::VIEM_STYLE_ROLE_PARAGRAPH);
        assert_eq!(
            document.origin,
            super::VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION
        );
        assert_ne!(
            document.capabilities & super::VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS,
            0
        );
        assert_eq!(
            document.capabilities & super::VIEM_STYLE_CAPABILITY_EDIT_PARENT,
            0
        );

        let document_properties = &properties[document.first_property as usize
            ..(document.first_property + document.property_count) as usize];
        let size = document_properties
            .iter()
            .find(|property| property.property == super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE)
            .unwrap();
        assert_ne!(size.flags & super::VIEM_STYLE_PROPERTY_DECLARED, 0);
        assert_eq!(size.declared.kind, super::VIEM_STYLE_VALUE_FLOAT);
        assert_eq!(size.declared.number, 14.0);
        assert_eq!(size.effective.number, 14.0);
        assert_eq!(
            size.contributor_kind,
            super::VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE
        );
        assert_eq!(
            style_arena_text(&strings, size.contributor_style_id),
            "Paragraph"
        );

        let families = document_properties
            .iter()
            .find(|property| {
                property.property == super::VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES
            })
            .unwrap();
        assert_eq!(families.declared.kind, super::VIEM_STYLE_VALUE_STRING_LIST);
        let family = value_items[families.declared.first_item as usize];
        assert_eq!(family.kind, super::VIEM_STYLE_VALUE_ITEM_STRING);
        assert_eq!(style_arena_text(&strings, family.string), crate::document::DEFAULT_FONT_FAMILY);

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
                property.property == super::VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES
            })
            .unwrap();
        assert_eq!(
            inherited_family.contributor_kind,
            super::VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE
        );
        assert_eq!(
            style_arena_text(&strings, inherited_family.contributor_style_id),
            "Paragraph"
        );
        assert!(inherited_family.dependency_count >= 2);

        // Every output remains untouched when just one capacity is short.
        let sentinel_definition = ViemStyleDefinitionV1 {
            role: u32::MAX,
            ..ViemStyleDefinitionV1::default()
        };
        let sentinel_property = ViemStylePropertyV1 {
            property: u32::MAX,
            ..ViemStylePropertyV1::default()
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
                viem_core_copy_style_sheet(
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
            ViemStatus::BufferTooSmall
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
                viem_core_copy_style_sheet(
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
            ViemStatus::StaleRevision
        );
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
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
            .block_style(&document.projection().style_sheet().base_paragraph)
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
        let mut code = document.projection().style_sheet().character_style(&"Code".into()).unwrap().clone();
        code.properties.size = Some(super::FontSize::Percentage(90));
        configure(&mut document, StyleDefinitionEdit::UpdateCharacter(code));

        let export = export_style_sheet(&document).unwrap();
        let kinds = export
            .properties
            .iter()
            .filter(|property| property.flags & super::VIEM_STYLE_PROPERTY_DECLARED != 0)
            .map(|property| property.declared.kind)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_FLOAT));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_UNSIGNED));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_BOOLEAN));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_COLOR));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_STRING));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_STRING_LIST));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_FONT_SLANT));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_WRITING_DIRECTION));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_LINE_SPACING));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_SCRIPT_POSITION));
        assert!(kinds.contains(&super::VIEM_STYLE_VALUE_PERCENTAGE));
        assert!(export.value_items.iter().any(|item| {
            item.kind == super::VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE
                && style_arena_text(&export.strings, item.string) == "kern"
                && item.unsigned_value == 1
        }));
    }

    struct PaintTestResponse {
        _carets: Box<[ViemClusterCaretStopV1]>,
        clusters: Box<[ViemShapedClusterV1]>,
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
        requests: *const ViemShapeRequestV1,
        request_count: u64,
        responses: *mut ViemShapeResponseV1,
        response_capacity: u64,
    ) -> u32 {
        let Ok(count) = usize::try_from(request_count) else {
            return ViemStatus::LengthOverflow as u32;
        };
        if request_count > response_capacity
            || context.is_null()
            || (count != 0 && (requests.is_null() || responses.is_null()))
        {
            return ViemStatus::InvalidArgument as u32;
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
            let metrics = ViemTextMetricsV1 {
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
                    ViemClusterCaretStopV1 {
                        text_offset: request.text_start,
                        inline_offset: 0.0,
                        affinity: VIEM_BOUNDARY_AFFINITY_DOWNSTREAM,
                    },
                    ViemClusterCaretStopV1 {
                        text_offset: request.text_end,
                        inline_offset: advance,
                        affinity: VIEM_BOUNDARY_AFFINITY_UPSTREAM,
                    },
                ]
                .into_boxed_slice();
                let has_render_run = request.purpose == VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA;
                let cluster = ViemShapedClusterV1 {
                    struct_size: VIEM_SHAPED_CLUSTER_V1_SIZE,
                    reserved: 0,
                    text_start: request.text_start,
                    text_end: request.text_end,
                    advance,
                    metrics,
                    typographic_bounds: ViemShapedBoundsV1 {
                        x: 0.0,
                        y: -10.0,
                        width: advance,
                        height: 13.0,
                    },
                    ink_bounds: ViemShapedBoundsV1 {
                        x: 0.0,
                        y: -10.0,
                        width: advance,
                        height: 13.0,
                    },
                    bidi_level: 0,
                    has_render_run: u32::from(has_render_run),
                    fallback_font: ViemUtf8Slice {
                        data: PAINT_TEST_FONT.as_ptr(),
                        length: PAINT_TEST_FONT.len() as u64,
                    },
                    caret_stops: carets.as_ptr(),
                    caret_stop_count: carets.len() as u64,
                    render_run: if has_render_run {
                        ViemRenderRunHandleV1 {
                            owner: request.render_run_owner,
                            identifier: index as u64 + 1,
                            metrics_generation: request.metrics_generation,
                            threading: request.render_run_threading,
                            reserved: 0,
                        }
                    } else {
                        ViemRenderRunHandleV1::default()
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
            *response = ViemShapeResponseV1 {
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
                ..ViemShapeResponseV1::default()
            };
        }
        ViemStatus::Ok as u32
    }

    unsafe extern "C" fn paint_test_retain_render_runs(
        _context: *mut c_void, _handles: *const ViemRenderRunHandleV1, _count: u64,
    ) -> *mut c_void { std::ptr::NonNull::<u8>::dangling().as_ptr().cast() }
    unsafe extern "C" fn paint_test_release_render_runs(_lease: *mut c_void) {}

    #[test]
    fn render_resource_lease_releases_once_after_final_snapshot_handle() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use crate::layout::RenderRunHandle;
        unsafe extern "C" fn release(context: *mut c_void) {
            // SAFETY: This test transferred one boxed Arc to the lease.
            let count = unsafe { Box::from_raw(context.cast::<Arc<AtomicUsize>>()) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        let count = Arc::new(AtomicUsize::new(0));
        let context = Box::into_raw(Box::new(Arc::clone(&count))) as usize;
        let lease: Arc<dyn Send + Sync> = Arc::new(super::CRenderResourceLease { context, release });
        let mut handle = RenderRunHandle::new(RenderRunOwner(1), 2, MetricsGeneration(3), RenderRunThreading::AnyThread);
        handle.retain_resource(Arc::clone(&lease));
        let retained_snapshot = handle.clone();
        drop(lease);
        drop(handle);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        std::thread::spawn(move || drop(retained_snapshot)).join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    fn paint_test_provider(
        storage: &mut PaintTestProviderStorage,
        environment: u64,
        owner: u64,
    ) -> CTextMeasurementProvider {
        CTextMeasurementProvider {
            context: (storage as *mut PaintTestProviderStorage) as usize,
            measurement_environment_id: MeasurementEnvironmentId(environment),
            threading: ProviderThreading::AnyWorker,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(owner),
                threading: RenderRunThreading::AnyThread,
            }),
            metrics_generation_callback: paint_test_metrics_generation,
            shape_batch_callback: paint_test_shape_batch,
            retain_render_runs_callback: Some(paint_test_retain_render_runs),
            release_render_runs_callback: Some(paint_test_release_render_runs),
        }
    }

    fn style_float_request(
        identity: super::ViemStyleSheetIdentityV1,
        style_id: &[u8],
        property: u32,
        value: f32,
    ) -> ViemStyleEditV1 {
        ViemStyleEditV1 {
            identity,
            namespace: super::VIEM_STYLE_NAMESPACE_BLOCK,
            operation: super::VIEM_STYLE_EDIT_SET_DECLARATION,
            property,
            style_id: ViemUtf8Slice {
                data: style_id.as_ptr(),
                length: style_id.len() as u64,
            },
            value: super::ViemStyleEditValueV1 {
                kind: super::VIEM_STYLE_VALUE_FLOAT,
                number: value,
                ..super::ViemStyleEditValueV1::default()
            },
            ..ViemStyleEditV1::default()
        }
    }

    fn current_style_identity(handle: super::ViemCoreHandle) -> super::ViemStyleSheetIdentityV1 {
        let mut info = ViemStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { viem_core_style_sheet_info(handle, &mut info) },
            ViemStatus::Ok
        );
        info.identity
    }

    #[test]
    fn ffi_percentage_font_sizes_remain_relative_through_edits_export_and_history() {
        use super::*;
        let document = Document::from_bytes_with_file_format(
            b"# heading `code`\n\nbody `code`".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        ).unwrap();
        let mut storage = PaintTestProviderStorage::default();
        let mut core = Core::new(document);
        let view = core.add_view(paint_test_provider(&mut storage, 911, 912), 400.0, 200.0);
        let handle = register_core(core).unwrap();
        let mut outcome = ViemCoreOutcomeV1::default();
        for (id, namespace, percent) in [
            (b"Heading1".as_slice(), VIEM_STYLE_NAMESPACE_BLOCK, 200),
            (b"Code".as_slice(), VIEM_STYLE_NAMESPACE_CHARACTER, 90),
        ] {
            let mut request = style_float_request(current_style_identity(handle), id,
                VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 0.0);
            request.namespace = namespace;
            request.value.kind = VIEM_STYLE_VALUE_PERCENTAGE;
            request.value.enum_value = percent;
            assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &request, &mut outcome) }, ViemStatus::Ok);
            assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
        }

        let assert_export = |base: f32| {
            let lease = checkout_core(handle).unwrap();
            let export = export_style_sheet(lease.core().document()).unwrap();
            for (id, percent, points) in [("Heading1", 200, base * 2.0), ("Code", 90, base * 0.9)] {
                let definition = export.definitions.iter().find(|definition|
                    style_arena_text(&export.strings, definition.stable_id) == id).unwrap();
                let property = export.properties[definition.first_property as usize
                    ..(definition.first_property + definition.property_count) as usize]
                    .iter().find(|property| property.property == VIEM_STYLE_PROPERTY_CHARACTER_SIZE).unwrap();
                assert_eq!(property.declared.kind, VIEM_STYLE_VALUE_PERCENTAGE);
                assert_eq!(property.declared.enum_value, percent);
                assert_eq!(property.declared.number, 0.0);
                assert_eq!(property.effective.kind, VIEM_STYLE_VALUE_FLOAT);
                assert!((property.effective.number - points).abs() < 0.0001);
                let dependencies = &export.dependencies[property.first_dependency as usize
                    ..(property.first_dependency + property.dependency_count) as usize];
                assert!(dependencies.iter().any(|dependency|
                    dependency.namespace == VIEM_STYLE_NAMESPACE_BLOCK
                        && style_arena_text(&export.strings, dependency.style_id) == "Paragraph"));
            }
        };
        assert_export(14.0);
        let base = style_float_request(current_style_identity(handle), b"Paragraph",
            VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 12.0);
        assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &base, &mut outcome) }, ViemStatus::Ok);
        assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
        assert_export(12.0);
        assert_eq!(unsafe { viem_core_view_undo(handle, view.0, &mut outcome) }, ViemStatus::Ok);
        assert_export(14.0);
        assert_eq!(unsafe { viem_core_view_redo(handle, view.0, &mut outcome) }, ViemStatus::Ok);
        assert_export(12.0);

        let identity = current_style_identity(handle);
        let mut invalid = style_float_request(identity, b"Heading1",
            VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 0.0);
        invalid.value.kind = VIEM_STYLE_VALUE_PERCENTAGE;
        for percent in [0, 9, 1001, u32::MAX] {
            invalid.value.enum_value = percent;
            assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &invalid, &mut outcome) }, ViemStatus::InvalidStyleValue);
            assert_eq!(current_style_identity(handle), identity);
        }
        invalid.value.enum_value = 90;
        invalid.value.number = 90.5;
        assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &invalid, &mut outcome) }, ViemStatus::InvalidStyleValue);
        assert_eq!(current_style_identity(handle), identity);
        invalid.value.number = 0.0;
        invalid.property = VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING;
        assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &invalid, &mut outcome) }, ViemStatus::InvalidStyleValue);
        assert_eq!(current_style_identity(handle), identity);
        invalid.property = VIEM_STYLE_PROPERTY_CHARACTER_SIZE;
        invalid.style_id = ViemUtf8Slice { data: b"Paragraph".as_ptr(), length: 9 };
        assert_eq!(unsafe { viem_core_view_edit_style(handle, view.0, &invalid, &mut outcome) }, ViemStatus::InvalidStyleValue);
        assert_eq!(current_style_identity(handle), identity);
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn ffi_native_line_navigation_validates_identity_before_cancelling_input() {
        let mut storage = PaintTestProviderStorage::default();
        let mut core = Core::new(Document::new("first\n  second\nthird"));
        let view = core.add_view(paint_test_provider(&mut storage, 501, 601), 240.0, 80.0);
        for key in ['3', 'i', 'X'] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
                .unwrap();
        }
        let document = core.document().id().0;
        let revision = core.document().revision().0;
        let handle = register_core(core).unwrap();
        let mut outcome = ViemCoreOutcomeV1::default();
        for (requested_document, requested_revision, expected) in [
            (document + 1, revision, ViemStatus::InvalidArgument),
            (document, revision - 1, ViemStatus::StaleRevision),
        ] {
            assert_eq!(
                unsafe {
                    super::viem_core_view_go_to_line(
                        handle, view.0, requested_document, requested_revision, 2, &mut outcome,
                    )
                },
                expected
            );
            assert_eq!(outcome, ViemCoreOutcomeV1::default());
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().command_state(view).unwrap().mode(), crate::command::Mode::Insert);
            assert_eq!(lease.core().document().text(), "Xfirst\n  second\nthird");
        }
        assert_eq!(
            unsafe {
                super::viem_core_view_go_to_line(
                    handle, view.0, document, revision, 2, ptr::null_mut(),
                )
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_go_to_line(
                    handle, view.0, document, revision, u64::MAX, &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(outcome.flags & super::VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().command_state(view).unwrap().mode(), crate::command::Mode::Normal);
            assert_eq!(lease.core().command_state(view).unwrap().cursor(), 16);
            assert_eq!(lease.core().document().revision().0, revision);
            assert_eq!(lease.core().document().text(), "Xfirst\n  second\nthird");
        }
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
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

        let mut presentation = super::ViemSemanticStylePresentationV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::VIEM_SEMANTIC_STYLE_STRONG,
                    &mut presentation,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(presentation.style, super::VIEM_SEMANTIC_STYLE_STRONG);
        assert_eq!(presentation.state, super::VIEM_SEMANTIC_STYLE_STATE_OFF);
        assert_eq!(
            presentation.flags,
            super::VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE | super::VIEM_SEMANTIC_STYLE_CAN_SET
        );
        assert_eq!(
            presentation.selection.kind,
            super::VIEM_LOGICAL_SELECTION_KIND_CHARACTER
        );
        assert_eq!(presentation.selection.text_start, 0);
        assert_eq!(presentation.selection.text_end, 5);

        let mut stale_selection = presentation.selection;
        stale_selection.state_identity[0] ^= 0xff;
        let stale_request = super::ViemSetSemanticStyleV1 {
            struct_size: super::VIEM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::VIEM_SEMANTIC_STYLE_STRONG,
            enabled: 1,
            reserved: 0,
            expected_selection: stale_selection,
        };
        let mut outcome = ViemCoreOutcomeV1 {
            document_revision: u64::MAX,
            ..ViemCoreOutcomeV1::default()
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &stale_request,
                    &mut outcome,
                )
            },
            ViemStatus::StaleRevision
        );
        assert_eq!(outcome, ViemCoreOutcomeV1::default());

        let set_strong = super::ViemSetSemanticStyleV1 {
            expected_selection: presentation.selection,
            ..stale_request
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_set_semantic_style(handle, view.0, &set_strong, &mut outcome)
            },
            ViemStatus::Ok
        );
        assert_ne!(outcome.flags & super::VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
        assert_ne!(outcome.flags & super::VIEM_OUTCOME_LAYOUT_CHANGED, 0);
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"**alpha** beta");
        }

        assert_eq!(
            unsafe {
                super::viem_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::VIEM_SEMANTIC_STYLE_STRONG,
                    &mut presentation,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(presentation.state, super::VIEM_SEMANTIC_STYLE_STATE_ON);
        assert_ne!(presentation.flags & super::VIEM_SEMANTIC_STYLE_CAN_CLEAR, 0);
        let clear_strong = super::ViemSetSemanticStyleV1 {
            struct_size: super::VIEM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::VIEM_SEMANTIC_STYLE_STRONG,
            enabled: 0,
            reserved: 0,
            expected_selection: presentation.selection,
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &clear_strong,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );

        assert_eq!(
            unsafe {
                super::viem_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::VIEM_SEMANTIC_STYLE_EMPHASIS,
                    &mut presentation,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(presentation.state, super::VIEM_SEMANTIC_STYLE_STATE_OFF);
        assert_ne!(presentation.flags & super::VIEM_SEMANTIC_STYLE_CAN_SET, 0);
        let set_emphasis = super::ViemSetSemanticStyleV1 {
            struct_size: super::VIEM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::VIEM_SEMANTIC_STYLE_EMPHASIS,
            enabled: 1,
            reserved: 0,
            expected_selection: presentation.selection,
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &set_emphasis,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"*alpha* beta");
        }
        assert_eq!(
            unsafe { super::viem_core_view_undo(handle, view.0, &mut outcome) },
            ViemStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"alpha beta");
        }
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
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
        let mut group = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &initial_identity,
                    &mut group,
                )
            },
            ViemStatus::Ok
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
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            31.0,
        );
        let mut outcome = ViemCoreOutcomeV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &size,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_ne!(outcome.flags & super::VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
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
                Some((31.0).into())
            );
        }

        // Every grouped edit is still exact-revision bound. A stale request
        // neither changes nor consumes the group.
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &size,
                    &mut outcome,
                )
            },
            ViemStatus::StaleRevision
        );
        let current_identity = current_style_identity(handle);
        let no_op = style_float_request(
            current_identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            31.0,
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &no_op,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(outcome.flags & super::VIEM_OUTCOME_DOCUMENT_CHANGED, 0);

        let invalid_role = style_float_request(
            current_identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
            7.0,
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &invalid_role,
                    &mut outcome,
                )
            },
            ViemStatus::IncompatibleStyleRole
        );

        let underline = ViemStyleEditV1 {
            identity: current_identity,
            namespace: super::VIEM_STYLE_NAMESPACE_BLOCK,
            operation: super::VIEM_STYLE_EDIT_SET_DECLARATION,
            property: super::VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            style_id: ViemUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::ViemStyleEditValueV1 {
                kind: super::VIEM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..super::ViemStyleEditValueV1::default()
            },
            ..ViemStyleEditV1::default()
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
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
                super::viem_core_view_edit_style_in_group(
                    handle,
                    second.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            ViemStatus::StyleEditGroupWrongOwner
        );
        let mut forged = group;
        forged.token += 1;
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &forged,
                    &underline,
                    &mut outcome,
                )
            },
            ViemStatus::InvalidStyleEditGroup
        );
        forged = group;
        forged.document_id += 1;
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &forged) },
            ViemStatus::InvalidArgument
        );
        let mut second_group = super::ViemStyleEditGroupV1::default();
        let latest_identity = current_style_identity(handle);
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    second.0,
                    &latest_identity,
                    &mut second_group,
                )
            },
            ViemStatus::StyleEditGroupActive
        );
        assert_eq!(second_group.token, 0);

        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &group) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &group) },
            ViemStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            ViemStatus::InvalidStyleEditGroup
        );

        assert_eq!(
            unsafe { super::viem_core_view_undo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            unsafe { super::viem_core_view_redo(handle, first.0, &mut outcome) },
            ViemStatus::Ok
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
            assert_eq!(style.character.size, Some((31.0).into()));
            assert_eq!(style.character.underline, Some(true));
        }

        assert_eq!(
            super::viem_core_view_remove(handle, first.0),
            ViemStatus::Ok
        );
        assert_eq!(
            super::viem_core_view_remove(handle, second.0),
            ViemStatus::Ok
        );
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
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
        let mut outcome = ViemCoreOutcomeV1::default();

        let identity = current_style_identity(handle);
        let mut empty = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(handle, first.0, &identity, &mut empty)
            },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &empty) },
            ViemStatus::Ok
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
        let mut first_group = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut first_group,
                )
            },
            ViemStatus::Ok
        );
        let size_32 = style_float_request(
            identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            32.0,
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &first_group,
                    &size_32,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &first_group) },
            ViemStatus::Ok
        );

        let identity = current_style_identity(handle);
        let mut second_group = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut second_group,
                )
            },
            ViemStatus::Ok
        );
        let size_33 = style_float_request(
            identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            33.0,
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &second_group,
                    &size_33,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );

        // A view-local option is still an unrelated coordinator event. It
        // closes the successful prefix before applying the option and makes
        // the old group capability invalid.
        assert_eq!(
            unsafe { super::viem_core_view_set_wrap(handle, first.0, 0, &mut outcome) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, first.0, &second_group) },
            ViemStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe { super::viem_core_view_undo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            Some((32.0).into())
        );
        assert_eq!(
            unsafe { super::viem_core_view_undo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            Some((32.0).into())
        );

        // Owner removal closes a committed group before dropping its
        // restoration state. The surviving view can undo the preserved unit.
        let identity = current_style_identity(handle);
        let mut removed_owner_group = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut removed_owner_group,
                )
            },
            ViemStatus::Ok
        );
        let size_34 = style_float_request(
            identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            34.0,
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &removed_owner_group,
                    &size_34,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(
            super::viem_core_view_remove(handle, first.0),
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe {
                super::viem_core_view_end_style_edit_group(handle, second.0, &removed_owner_group)
            },
            ViemStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe { super::viem_core_view_undo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            Some((34.0).into())
        );

        // Core destruction owns final cleanup. The capability cannot be used
        // after the core handle is destroyed.
        let identity = current_style_identity(handle);
        let mut destroy_group = super::ViemStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_begin_style_edit_group(
                    handle,
                    second.0,
                    &identity,
                    &mut destroy_group,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
        assert_eq!(
            unsafe { super::viem_core_view_end_style_edit_group(handle, second.0, &destroy_group) },
            ViemStatus::InvalidHandle
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

        let mut info = ViemStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { viem_core_style_sheet_info(handle, &mut info) },
            ViemStatus::Ok
        );
        let heading = b"Heading1";
        let first_request = style_float_request(
            info.identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            30.0,
        );
        let mut outcome = ViemCoreOutcomeV1::default();
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style(handle, first.0, &first_request, &mut outcome)
            },
            ViemStatus::Ok
        );
        assert_ne!(outcome.flags & super::VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
        assert_ne!(outcome.flags & super::VIEM_OUTCOME_LAYOUT_CHANGED, 0);
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
            assert_eq!(style.character.size, Some((30.0).into()));
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
                super::viem_core_view_edit_style(handle, second.0, &first_request, &mut outcome)
            },
            ViemStatus::StaleRevision
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
            unsafe { viem_core_style_sheet_info(handle, &mut info) },
            ViemStatus::Ok
        );
        let bad_role = style_float_request(
            info.identity,
            heading,
            super::VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
            4.0,
        );
        assert_eq!(
            unsafe { super::viem_core_view_edit_style(handle, first.0, &bad_role, &mut outcome) },
            ViemStatus::IncompatibleStyleRole
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

        let self_parent = ViemStyleEditV1 {
            identity: info.identity,
            namespace: super::VIEM_STYLE_NAMESPACE_BLOCK,
            operation: super::VIEM_STYLE_EDIT_SET_PARENT,
            style_id: ViemUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::ViemStyleEditValueV1 {
                kind: super::VIEM_STYLE_VALUE_STRING,
                text: ViemUtf8Slice {
                    data: heading.as_ptr(),
                    length: heading.len() as u64,
                },
                ..super::ViemStyleEditValueV1::default()
            },
            ..ViemStyleEditV1::default()
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_edit_style(handle, second.0, &self_parent, &mut outcome)
            },
            ViemStatus::StyleInheritanceCycle
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
        let underline = ViemStyleEditV1 {
            identity: info.identity,
            namespace: super::VIEM_STYLE_NAMESPACE_BLOCK,
            operation: super::VIEM_STYLE_EDIT_SET_DECLARATION,
            property: super::VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            style_id: ViemUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::ViemStyleEditValueV1 {
                kind: super::VIEM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..super::ViemStyleEditValueV1::default()
            },
            ..ViemStyleEditV1::default()
        };
        assert_eq!(
            unsafe { super::viem_core_view_edit_style(handle, second.0, &underline, &mut outcome) },
            ViemStatus::Ok
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
            unsafe { super::viem_core_view_undo(handle, first.0, &mut outcome) },
            ViemStatus::Ok
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
            assert_eq!(style.character.size, Some((30.0).into()));
            assert_eq!(style.character.underline, None);
            assert_eq!(lease.core().document().source_bytes(), source);
        }
        assert_eq!(
            unsafe { super::viem_core_view_redo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            unsafe { viem_core_style_sheet_info(handle, &mut info) },
            ViemStatus::Ok
        );
        let begin_composition = super::ViemCompositionBeginV1 {
            struct_size: super::VIEM_COMPOSITION_BEGIN_V1_SIZE,
            reserved: 0,
            document_revision: info.identity.document_revision,
            replacement_start: 0,
            replacement_end: 0,
        };
        assert_eq!(
            unsafe {
                super::viem_core_view_composition_begin(
                    handle,
                    second.0,
                    &begin_composition,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        let display_name = b"Chapter Heading";
        let rename = ViemStyleEditV1 {
            identity: info.identity,
            namespace: super::VIEM_STYLE_NAMESPACE_BLOCK,
            operation: super::VIEM_STYLE_EDIT_SET_DISPLAY_NAME,
            style_id: ViemUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::ViemStyleEditValueV1 {
                kind: super::VIEM_STYLE_VALUE_STRING,
                text: ViemUtf8Slice {
                    data: display_name.as_ptr(),
                    length: display_name.len() as u64,
                },
                ..super::ViemStyleEditValueV1::default()
            },
            ..ViemStyleEditV1::default()
        };
        assert_eq!(
            unsafe { super::viem_core_view_edit_style(handle, first.0, &rename, &mut outcome) },
            ViemStatus::Ok
        );
        assert_ne!(
            outcome.flags & super::VIEM_OUTCOME_HAS_COMPOSITION_CHANGES,
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
            unsafe { super::viem_core_view_undo(handle, second.0, &mut outcome) },
            ViemStatus::Ok
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
            unsafe { super::viem_core_view_redo(handle, first.0, &mut outcome) },
            ViemStatus::Ok
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

        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn ffi_boundary_contains_rust_panics() {
        let status = ffi_boundary(|| -> Result<(), ViemStatus> {
            panic!("deliberate ABI-boundary test panic")
        });
        assert_eq!(status, ViemStatus::Panic);
    }

    #[test]
    fn panic_returns_checked_out_handles_and_busy_destroy_is_retryable() {
        let core_handle = register_core(Core::new(Document::new("text"))).unwrap();
        let status = ffi_boundary(|| -> Result<(), ViemStatus> {
            let _lease = checkout_core(core_handle)?;
            assert_eq!(
                viem_core_destroy(core_handle),
                ViemStatus::CoreBusy,
                "destroy must retain a checked-out core"
            );
            panic!("panic while a core turn is checked out")
        });
        assert_eq!(status, ViemStatus::Panic);
        drop(checkout_core(core_handle).unwrap());
        assert_eq!(viem_core_destroy(core_handle), ViemStatus::Ok);
        assert!(matches!(
            checkout_core(core_handle),
            Err(ViemStatus::InvalidHandle)
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
            measurement_environment_id: MeasurementEnvironmentId(41),
            threading: ProviderThreading::AnyWorker,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(42),
                threading: RenderRunThreading::AnyThread,
            }),
            metrics_generation_callback: paint_test_metrics_generation,
            shape_batch_callback: paint_test_shape_batch,
            retain_render_runs_callback: Some(paint_test_retain_render_runs),
            release_render_runs_callback: Some(paint_test_release_render_runs),
        };
        let mut core = Core::new(document);
        let view = core.add_view(provider, 800.0, 600.0);
        assert!(core.layout(view).unwrap().snapshot().is_some());
        let handle = register_core(core).unwrap();

        let mut info = ViemLayoutPaintInfoV1::default();
        assert_eq!(
            unsafe { viem_core_view_layout_paint_info(handle, view.0, &mut info) },
            ViemStatus::Ok
        );
        assert_eq!(info.struct_size, VIEM_LAYOUT_PAINT_INFO_V1_SIZE);
        assert_eq!(
            info.canvas_background,
            super::ViemRgbaV1 {
                red: canvas.red,
                green: canvas.green,
                blue: canvas.blue,
                alpha: canvas.alpha,
            }
        );
        assert_eq!(info.default_paint.struct_size, VIEM_TEXT_PAINT_V1_SIZE);
        assert_eq!(
            info.default_paint.flags,
            VIEM_TEXT_PAINT_HAS_BACKGROUND | VIEM_TEXT_PAINT_UNDERLINE
        );
        assert_eq!(info.default_paint.foreground.red, default_foreground.red);
        assert_eq!(
            info.default_paint.background.alpha,
            default_background.alpha
        );
        assert!(info.paint_run_count > 0);

        let mut copied = ViemLayoutPaintInfoV1::default();
        assert_eq!(
            unsafe {
                viem_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    ptr::null_mut(),
                    0,
                    &mut copied,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(copied, info);

        let mut runs = vec![ViemPaintStyleRunV1::default(); info.paint_run_count as usize];
        runs[0].text_start = u64::MAX;
        assert_eq!(
            unsafe {
                viem_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    runs.as_mut_ptr(),
                    runs.len().saturating_sub(1) as u64,
                    &mut copied,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(copied, info);
        assert_eq!(runs[0].text_start, u64::MAX, "copy must not be partial");
        assert_eq!(
            unsafe {
                viem_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    runs.as_mut_ptr(),
                    runs.len() as u64,
                    &mut copied,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(copied, info);
        assert!(runs
            .windows(2)
            .all(|pair| pair[0].text_end <= pair[1].text_start));
        for run in &runs {
            assert_eq!(run.struct_size, VIEM_PAINT_STYLE_RUN_V1_SIZE);
            assert!(run.text_start < run.text_end);
            assert_eq!(run.paint.struct_size, VIEM_TEXT_PAINT_V1_SIZE);
            assert_eq!(
                run.paint.flags,
                VIEM_TEXT_PAINT_HAS_BACKGROUND | VIEM_TEXT_PAINT_STRIKETHROUGH
            );
            assert_eq!(run.paint.foreground.blue, run_foreground.blue);
            assert_eq!(run.paint.background.alpha, run_background.alpha);
        }

        assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn provider_request_supplies_paragraph_direction() {
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
            storage.request(&request).paragraph_base_direction,
            VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT
        );
    }
}

/// Read a view's line-command domain (0 visual, 1 physical source).
/// # Safety
/// out_mode must identify one aligned writable u32.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_line_mode(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_mode: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_mode.is_null() || (out_mode as usize) % std::mem::align_of::<u32>() != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        let mode = with_core_mut(handle, |core| {
            core.command_state(crate::coordinator::ViewId(view))
                .map(|state| state.line_mode() as u32)
                .ok_or(ViemStatus::InvalidView)
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
pub unsafe extern "C" fn viem_core_view_set_line_mode(
    handle: ViemCoreHandle,
    view: ViemViewId,
    mode: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe {
            clear_outcome(out_outcome)?;
        }
        let mode = match mode {
            0 => crate::command::LineMode::Visual,
            1 => crate::command::LineMode::PhysicalSource,
            _ => return Err(ViemStatus::InvalidArgument),
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
pub unsafe extern "C" fn viem_core_view_paragraph_flow(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_enabled: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_enabled.is_null() || (out_enabled as usize) % std::mem::align_of::<u32>() != 0 {
            return Err(ViemStatus::InvalidArgument);
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
pub unsafe extern "C" fn viem_core_view_set_paragraph_flow(
    handle: ViemCoreHandle,
    view: ViemViewId,
    enabled: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe {
            clear_outcome(out_outcome)?;
        }
        if enabled > 1 {
            return Err(ViemStatus::InvalidArgument);
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
pub extern "C" fn viem_core_view_set_smart_quotes(
    handle: ViemCoreHandle,
    view: ViemViewId,
    enabled: u32,
) -> ViemStatus {
    ffi_boundary(|| {
        if enabled > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetSmartQuotes(enabled != 0)).map(|_| ())
        })
    })
}

pub const VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT: u32 = 1;
pub const VIEM_LINE_LOCATION_FRAGMENT_EXACT: u32 = 2;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViemViewLineLocationV1 {
    pub struct_size: u32,
    pub mode: u32,
    pub flags: u32,
    pub reserved: u32,
    pub line: u64,
    pub column: u64,
    pub hard_line: u64,
    pub fragment: u64,
}
pub const VIEM_VIEW_LINE_LOCATION_V1_SIZE: u32 =
    std::mem::size_of::<ViemViewLineLocationV1>() as u32;
/// Report one-based line/column and the exact hard-line/fragment fallback.
/// line is zero when the global visual row number is not yet materialized.
/// # Safety
/// out_location must identify one aligned writable V1 location record.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_line_location(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_location: *mut ViemViewLineLocationV1,
) -> ViemStatus {
    ffi_boundary(|| {
        if out_location.is_null()
            || (out_location as usize) % std::mem::align_of::<ViemViewLineLocationV1>() != 0
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let location = with_core_mut(handle, |core| {
            core.line_location(ViewId(view)).map_err(core_status)
        })?;
        let result = ViemViewLineLocationV1 {
            struct_size: VIEM_VIEW_LINE_LOCATION_V1_SIZE,
            mode: location.mode as u32,
            flags: VIEM_LINE_LOCATION_FRAGMENT_EXACT
                | if location.line.is_some() {
                    VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT
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
pub extern "C" fn viem_core_set_read_only(
    handle: ViemCoreHandle,
    document: u64,
    revision: u64,
    read_only: u32,
) -> ViemStatus {
    ffi_boundary(|| {
        let value = match read_only {
            0 => false,
            1 => true,
            _ => return Err(ViemStatus::InvalidArgument),
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
pub extern "C" fn viem_core_mark_recovered(
    handle: ViemCoreHandle,
    document: u64,
    revision: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        with_core_mut(handle, |core| {
            core.mark_recovered(crate::document::DocumentId(document), Revision(revision))
                .map_err(core_status)
        })
    })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemSelectedStylesInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
    pub paragraph_id_bytes: u64,
    pub character_id_bytes: u64,
}

/// Copy selected named style IDs. Code includes the displayed automatic
/// character style; other formats exclude automatic source syntax styles.
/// # Safety
/// Output regions must be aligned, writable, and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_selected_styles_export(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected_revision: u64,
    out_info: *mut ViemSelectedStylesInfoV1,
    out_utf8: *mut u8,
    capacity: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let info_region = typed_pointer_region(out_info, 1)?;
        let bytes_region = typed_pointer_region(out_utf8, capacity)?;
        if regions_overlap(info_region, bytes_region) {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe {
            out_info.write(ViemSelectedStylesInfoV1::default());
        }
        let (info, bytes) = with_core(handle, |core| {
            if core.document().revision().0 != expected_revision {
                return Err(ViemStatus::StaleRevision);
            }
            let selected = core
                .selected_named_styles(ViewId(view))
                .map_err(core_status)?;
            let paragraph = selected.paragraph.map(|id| id.0).unwrap_or_default();
            let character = selected.character.map(|id| id.0).unwrap_or_default();
            let info = ViemSelectedStylesInfoV1 {
                struct_size: size_of::<ViemSelectedStylesInfoV1>() as u32,
                flags: u32::from(selected.paragraph_mixed)
                    | (u32::from(selected.character_mixed) << 1)
                    | (u32::from(selected.has_bullets) << 2)
                    | (u32::from(selected.has_numbering) << 3)
                    | (u32::from(selected.has_non_list) << 4),
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
            return Err(ViemStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_utf8, bytes.len());
            }
        }
        Ok(())
    })
}

pub type ViemStyleDefaultsDiagnosticCallback =
    Option<unsafe extern "C" fn(*mut c_void, *const u8, u64)>;

/// # Safety
/// Output and required-length records must be writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_theme_default_json(preset: u32, output: *mut u8, capacity: u64, required: *mut u64) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(output, capacity)?, typed_pointer_region(required, 1)?])?;
        let bytes = crate::document::theme::default_json(preset).map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe { required.write(bytes.len() as u64); }
        if capacity < bytes.len() as u64 { return Err(ViemStatus::BufferTooSmall); }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()); }
        Ok(())
    })
}

/// # Safety
/// JSON is readable for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn viem_theme_validate_json(json: *const u8, length: u64) -> ViemStatus {
    ffi_boundary(|| {
        if length > crate::document::theme::MAX_THEME_BYTES as u64 { return Err(ViemStatus::InvalidArgument); }
        crate::document::theme::validate_json(unsafe { input_bytes(json, length)? }).map_err(|_| ViemStatus::InvalidArgument)
    })
}

/// # Safety
/// Name is a readable UTF-8 slice for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn viem_theme_validate_name(name: *const u8, length: u64) -> ViemStatus {
    ffi_boundary(|| {
        if length > 128 { return Err(ViemStatus::InvalidArgument); }
        let name = str::from_utf8(unsafe { input_bytes(name, length)? }).map_err(|_| ViemStatus::InvalidUtf8)?;
        crate::document::theme::validate_name(name).map_err(|_| ViemStatus::InvalidArgument)
    })
}

/// Replace live application defaults without editing source or document history.
/// # Safety
/// Input and callback follow `viem_core_initialize_style_defaults`'s contract.
#[no_mangle]
pub unsafe extern "C" fn viem_core_replace_style_defaults(handle: ViemCoreHandle, expected_revision: u64, json: *const u8, length: u64, diagnostic: ViemStyleDefaultsDiagnosticCallback, context: *mut c_void) -> ViemStatus {
    ffi_boundary(|| {
        let bytes = unsafe { input_bytes(json, length)? };
        let result = with_core_mut(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            Ok(core.replace_style_defaults(bytes))
        })?;
        let (messages, status) = match result {
            Ok(messages) => (messages, Ok(())),
            Err(error) => (vec![error.to_string()], Err(ViemStatus::InvalidArgument)),
        };
        if let Some(callback) = diagnostic {
            for message in messages { unsafe { callback(context, message.as_ptr(), message.len() as u64); } }
        }
        status
    })
}

/// Load usable JSON defaults before a core has views or edits. No source mutation.
/// Diagnostics describe ignored settings and run after releasing the core lease.
///
/// # Safety
/// JSON must be readable for this call. The callback and context must be valid
/// for synchronous invocation and must not unwind through C. Diagnostic bytes
/// are borrowed only for the duration of the callback.
#[no_mangle]
pub unsafe extern "C" fn viem_core_initialize_style_defaults(
    handle: ViemCoreHandle,
    expected_revision: u64,
    json: *const u8,
    length: u64,
    diagnostic: ViemStyleDefaultsDiagnosticCallback,
    context: *mut c_void,
) -> ViemStatus {
    ffi_boundary(|| {
        let bytes = unsafe { input_bytes(json, length)? };
        let result = with_core_mut(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            Ok(core.initialize_style_defaults(bytes))
        })?;
        let (messages, status) = match result {
            Ok(messages) => (messages, Ok(())),
            Err(error) => (vec![error.to_string()], Err(ViemStatus::InvalidArgument)),
        };
        if let Some(callback) = diagnostic {
            for message in messages {
                unsafe { callback(context, message.as_ptr(), message.len() as u64) };
            }
        }
        status
    })
}

/// # Safety
/// filename is a readable UTF-8 slice for this call; detection is bounded.
#[no_mangle]
pub unsafe extern "C" fn viem_core_initialize_code_detection(handle:ViemCoreHandle, filename:*const u8, length:u64, allow_auto_code:u8)->ViemStatus {
    ffi_boundary(|| {
        if allow_auto_code>1 || length>16*1024 {return Err(ViemStatus::InvalidArgument);}
        let name=str::from_utf8(unsafe{input_bytes(filename,length)?}).map_err(|_|ViemStatus::InvalidUtf8)?;
        if name.contains('\0') {return Err(ViemStatus::InvalidArgument);}
        with_core_mut(handle,|core|core.initialize_code_detection(name,allow_auto_code!=0).map_err(|_|ViemStatus::InvalidArgument))
    })
}

/// # Safety
/// The directory is a readable UTF-8 slice. This records configuration only;
/// package loading never takes place on this call's thread.
#[no_mangle]
pub unsafe extern "C" fn viem_core_configure_syntax(handle:ViemCoreHandle,directory:*const u8,length:u64)->ViemStatus {
    ffi_boundary(|| {
        if length>16*1024 {return Err(ViemStatus::InvalidArgument);}
        let path=str::from_utf8(unsafe{input_bytes(directory,length)?}).map_err(|_|ViemStatus::InvalidUtf8)?;
        if path.contains('\0') {return Err(ViemStatus::InvalidArgument);}
        with_core_mut(handle,|core|{core.configure_syntax(path);Ok(())})
    })
}

/// # Safety
/// JSON is a readable byte slice for this call. Empty input clears the table.
#[no_mangle]
pub unsafe extern "C" fn viem_core_set_code_filename_associations_json(handle:ViemCoreHandle,json:*const u8,length:u64)->ViemStatus {
    ffi_boundary(|| {
        if length>128*1024 {return Err(ViemStatus::InvalidArgument);}
        let bytes=unsafe{input_bytes(json,length)?};
        let associations=if bytes.is_empty(){Vec::new()}else{serde_json::from_slice::<Vec<crate::document::syntax::detection::FilenameAssociation>>(bytes).map_err(|_|ViemStatus::InvalidArgument)?};
        with_core_mut(handle,|core|core.set_code_filename_associations(associations).map_err(|_|ViemStatus::InvalidArgument))
    })
}

/// # Safety
/// selection is Automatic=0, None=1, or Language=2. Language is bounded UTF-8.
#[no_mangle]
pub unsafe extern "C" fn viem_core_set_code_language(handle:ViemCoreHandle,selection:u32,language:*const u8,length:u64)->ViemStatus {
    ffi_boundary(|| {
        use crate::document::syntax::detection::LanguageSelection;
        if length>128 {return Err(ViemStatus::InvalidArgument);}
        let value=str::from_utf8(unsafe{input_bytes(language,length)?}).map_err(|_|ViemStatus::InvalidUtf8)?;
        let selection=match selection {
            0 if value.is_empty()=>LanguageSelection::Automatic,
            1 if value.is_empty()=>LanguageSelection::None,
            2 if !value.is_empty() && value.bytes().all(|b|b.is_ascii_alphanumeric() || b"_+.#-".contains(&b))=>LanguageSelection::Language(value.to_owned()),
            _=>return Err(ViemStatus::InvalidArgument),
        };
        with_core_mut(handle,|core|{core.set_code_language(selection);Ok(())})
    })
}

/// Set the application default for `textwidth` on this buffer. The value must
/// be positive. An explicit `:set textwidth` override in the buffer survives;
/// buffers still inheriting the default adopt it immediately. This never
/// changes source, dirty state, or undo history.
#[no_mangle]
pub extern "C" fn viem_core_set_text_width_default(handle: ViemCoreHandle, width: u32) -> ViemStatus {
    ffi_boundary(|| {
        if width == 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            core.set_text_width_default(width);
            Ok(())
        })
    })
}

/// # Safety
/// filename is bounded UTF-8; empty input preserves the existing filename.
#[no_mangle]
pub unsafe extern "C" fn viem_core_redetect_code_language(handle:ViemCoreHandle,filename:*const u8,length:u64)->ViemStatus {
    ffi_boundary(|| {
        if length>16*1024 {return Err(ViemStatus::InvalidArgument);}
        let name=str::from_utf8(unsafe{input_bytes(filename,length)?}).map_err(|_|ViemStatus::InvalidUtf8)?;
        if name.contains('\0') {return Err(ViemStatus::InvalidArgument);}
        with_core_mut(handle,|core|{core.redetect_code_language((!name.is_empty()).then_some(name));Ok(())})
    })
}

/// # Safety
/// changed points to one writable byte. Does no provider work and never waits.
#[no_mangle]
pub unsafe extern "C" fn viem_core_poll_syntax(handle:ViemCoreHandle,changed:*mut u8)->ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(changed,1)?;
        let value=with_core_mut(handle,|core|Ok(core.poll_syntax()))?;
        unsafe{changed.write(u8::from(value));} Ok(())
    })
}

/// # Safety
/// Standard two-pass UTF-8 output, disjoint from the required-length record.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_syntax_diagnostics(handle:ViemCoreHandle,output:*mut u8,capacity:u64,required:*mut u64)->ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(output,capacity)?,typed_pointer_region(required,1)?])?;
        let text=with_core(handle,|core|Ok(core.syntax_diagnostics()))?;
        unsafe{required.write(text.len() as u64);}
        if capacity < text.len() as u64 {return Err(ViemStatus::BufferTooSmall);}
        unsafe{copy_output(text.as_bytes(),output);} Ok(())
    })
}

/// Read-only two-pass JSON array of unique, sorted names from accepted syntax
/// runs, including names without Code stylesheet definitions. No parsing,
/// publication, or document-wide text scan occurs during this query.
///
/// # Safety
/// Output storage and the required-length record must be aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_syntax_style_names(
    handle: ViemCoreHandle,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        let bytes = with_core(handle, |core| {
            serde_json::to_vec(core.syntax_style_names()).map_err(|_| ViemStatus::CoreFailure)
        })?;
        unsafe { required.write(bytes.len() as u64) };
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe { copy_output(&bytes, output) };
        Ok(())
    })
}

/// Resolve an authored link at an exact formatted snapshot point. No link is
/// represented by found=0; an empty destination remains a link with found=1.
/// # Safety
/// All output regions must be aligned, writable, and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_link_destination(
    handle: ViemCoreHandle,
    document_id: u64,
    revision: u64,
    text_offset: u64,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
    found: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
            typed_pointer_region(found, 1)?,
        ])?;
        let destination = with_core(handle, |core| {
            let document = core.document();
            if document.id().0 != document_id { return Err(ViemStatus::InvalidArgument); }
            validate_revision(document, revision)?;
            let offset = usize::try_from(text_offset).map_err(|_| ViemStatus::LengthOverflow)?;
            let point = document.text_point(offset).map_err(document_status)?;
            document.link_at(point).map_err(document_status)
        })?;
        let bytes = destination.as_deref().unwrap_or("").as_bytes();
        unsafe {
            required.write(bytes.len() as u64);
            found.write(u8::from(destination.is_some()));
        }
        if capacity < bytes.len() as u64 { return Err(ViemStatus::BufferTooSmall); }
        unsafe { copy_output(bytes, output); }
        Ok(())
    })
}

/// Two-pass JSON export of sparse defaults plus explicit document declarations.
#[no_mangle]
pub unsafe extern "C" fn viem_core_export_style_defaults(
    handle: ViemCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        if required.is_null() {
            return Err(ViemStatus::NullPointer);
        }
        unsafe { required.write(0) };
        let bytes = with_core(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            core.document()
                .export_style_defaults()
                .map_err(|_| ViemStatus::InvalidArgument)
        })?;
        unsafe { required.write(bytes.len() as u64) };
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            if output.is_null() {
                return Err(ViemStatus::NullPointer);
            }
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        }
        Ok(())
    })
}

/// Directed, revision-checked command prompt selection (offsets exclude the prompt).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemCommandLineSelectionV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub anchor_utf8_offset: u64,
    pub active_utf8_offset: u64,
}

/// # Safety
/// `expected` and `out_selection` must be aligned and disjoint readable/writable records.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_command_line_selection(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemCommandLineIdentityV1,
    out_selection: *mut ViemCommandLineSelectionV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let a = typed_pointer_region(expected, 1)?;
        let b = typed_pointer_region(out_selection, 1)?;
        if regions_overlap(a, b) {
            return Err(ViemStatus::InvalidArgument);
        }
        let expected = unsafe { read_command_line_identity(expected)? };
        unsafe {
            out_selection.write(ViemCommandLineSelectionV1::default());
        }
        let selection = with_core(handle, |core| {
            validate_command_line_identity(
                expected,
                export_command_line(core, ViewId(view))?.info.identity,
            )?;
            let snapshot = core
                .command_state(ViewId(view))
                .and_then(|s| s.command_line_snapshot());
            Ok(ViemCommandLineSelectionV1 {
                struct_size: size_of::<ViemCommandLineSelectionV1>() as u32,
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
pub unsafe extern "C" fn viem_core_view_edit_command_line(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemCommandLineIdentityV1,
    operation: u32,
    start: u64,
    end: u64,
    text: *const u8,
    length: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let a = typed_pointer_region(expected, 1)?;
        let b = typed_pointer_region(text, length)?;
        let c = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(a, b) || regions_overlap(a, c) || regions_overlap(b, c) {
            return Err(ViemStatus::InvalidArgument);
        }
        let expected = unsafe { read_command_line_identity(expected)? };
        let text = std::str::from_utf8(unsafe { input_bytes(text, length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?
            .to_owned();
        let start = usize::try_from(start).map_err(|_| ViemStatus::LengthOverflow)?;
        let end = usize::try_from(end).map_err(|_| ViemStatus::LengthOverflow)?;
        let action = match operation {
            0 if text.is_empty() => crate::command::CommandLineEditAction::Select {
                anchor: start,
                active: end,
            },
            1 => crate::command::CommandLineEditAction::Replace {
                range: start..end,
                text,
            },
            _ => return Err(ViemStatus::InvalidArgument),
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
                .ok_or(ViemStatus::InvalidArgument)?;
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
pub unsafe extern "C" fn viem_core_view_set_format_with_effects(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetFormatV1,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_SET_FORMAT_V1_SIZE || request.reserved != 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe {
            clear_outcome(out_outcome)?;
            out_effects.write(0);
        }
        let target = parse_format(request.format)?;
        let operation = match request.operation {
            VIEM_FORMAT_OPERATION_REINTERPRET => FormatOperation::Reinterpret,
            VIEM_FORMAT_OPERATION_CONVERT => FormatOperation::Convert,
            _ => return Err(ViemStatus::InvalidArgument),
        };
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
                        operation,
                    },
                )
                .map_err(core_status)?;
            let summary = summarize_core_outcome(core, view, Some(&outcome))?;
            let effects = OwnedEffectBatch::from_command(
                core.document(),
                core.command_state(view).ok_or(ViemStatus::InvalidView)?,
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
pub unsafe extern "C" fn viem_core_view_set_encoding_with_effects(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetEncodingV1,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_SET_ENCODING_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe {
            clear_outcome(out_outcome)?;
            out_effects.write(0);
        }
        let target = parse_encoding(request.encoding)?.ok_or(ViemStatus::InvalidEncoding)?;
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
                core.command_state(view).ok_or(ViemStatus::InvalidView)?,
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
pub unsafe extern "C" fn viem_core_copy_hard_line_source_bytes(
    handle: ViemCoreHandle,
    document: u64,
    revision: u64,
    first_line: u64,
    end_line: u64,
    output: *mut u8,
    capacity: u64,
    out_required: *mut u64,
    out_complete: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(out_required, 1)?,
            typed_pointer_region(out_complete, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        unsafe {
            out_required.write(0);
            out_complete.write(0);
        }
        let (bytes, complete) = with_core(handle, |core| {
            let doc = core.document();
            if doc.id() != DocumentId(document) {
                return Err(ViemStatus::InvalidArgument);
            }
            validate_revision(doc, revision)?;
            let range = doc
                .source_byte_range_for_hard_lines(
                    checked_length(first_line)?..checked_length(end_line)?,
                )
                .map_err(|_| ViemStatus::PolicyRequired)?;
            let complete = range.start == 0 && range.end == doc.source_byte_len();
            Ok((doc.source_bytes()[range].to_vec(), complete))
        })?;
        unsafe {
            out_required.write(bytes.len() as u64);
            out_complete.write(u32::from(complete));
        }
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
            }
        }
        Ok(())
    })
}

unsafe fn read_command_turn_context(
    pointer: *const ViemCommandTurnContextV2,
    forbidden_outputs: &[(usize, usize)],
) -> Result<ClipboardCommandContext, ViemStatus> {
    let region = typed_pointer_region(pointer, 1)?;
    if forbidden_outputs.iter().any(|output| regions_overlap(region, *output)) { return Err(ViemStatus::InvalidArgument); }
    let context = unsafe { pointer.read() };
    if context.struct_size < VIEM_COMMAND_TURN_CONTEXT_V2_SIZE || context.reserved != 0 || context.clipboard_count > 2 { return Err(ViemStatus::InvalidArgument); }
    let entries_region = typed_pointer_region(context.clipboards, context.clipboard_count)?;
    if forbidden_outputs.iter().any(|output| regions_overlap(entries_region, *output)) { return Err(ViemStatus::InvalidArgument); }
    let entries = if context.clipboard_count == 0 { Vec::new() } else {
        unsafe { slice::from_raw_parts(context.clipboards, checked_length(context.clipboard_count)?) }.to_vec()
    };
    let mut result = ClipboardCommandContext::new();
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        if entry.struct_size < VIEM_CLIPBOARD_TURN_ENTRY_V2_SIZE || entry.reserved != 0
            || entry.flags & !(VIEM_CLIPBOARD_TURN_HAS_READ | VIEM_CLIPBOARD_TURN_WRITABLE) != 0 { return Err(ViemStatus::InvalidArgument); }
        let target = parse_clipboard_target(entry.target)?;
        if !seen.insert(target) { return Err(ViemStatus::InvalidArgument); }
        for region in [typed_pointer_region(entry.plain_text.data, entry.plain_text.length)?,typed_pointer_region(entry.fragment_json.data, entry.fragment_json.length)?] {
            if forbidden_outputs.iter().any(|output| regions_overlap(region,*output)) { return Err(ViemStatus::InvalidArgument); }
        }
        if entry.flags & VIEM_CLIPBOARD_TURN_HAS_READ != 0 {
            let text = str::from_utf8(unsafe { input_bytes(entry.plain_text.data,entry.plain_text.length)? }).map_err(|_| ViemStatus::InvalidUtf8)?.to_owned();
            let content = if entry.fragment_json.length == 0 { ClipboardContent::from_plain_text(text) } else {
                let bytes = unsafe { input_bytes(entry.fragment_json.data,entry.fragment_json.length)? };
                let register = str::from_utf8(bytes).ok()
                    .and_then(|json| crate::document::ClipboardFragment::from_json(json,&text).ok())
                    .and_then(|fragment| crate::command::RegisterValue::from_clipboard_fragment(fragment).ok());
                ClipboardContent::try_new(text,register).map_err(|_| ViemStatus::InvalidArgument)?
            };
            result = result.with_read(ClipboardSnapshot::new(target,ClipboardGeneration(entry.generation),content));
        } else if entry.generation != 0 || entry.plain_text.length != 0 || entry.fragment_json.length != 0 { return Err(ViemStatus::InvalidArgument); }
        if entry.flags & VIEM_CLIPBOARD_TURN_WRITABLE != 0 { result = result.with_write(target); }
    }
    Ok(result)
}

/// Copy versioned source/style clipboard JSON for an exact formatted range.
/// # Safety
/// Inputs/outputs must be valid, aligned where typed, and non-overlapping.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_clipboard_json(
    handle: ViemCoreHandle,
    request: *const ViemFormattedUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request,1)?;
        let output_region = typed_pointer_region(output,output_capacity)?;
        let required_region = typed_pointer_region(out_required,1)?;
        if regions_overlap(request_region,output_region) || regions_overlap(request_region,required_region) || regions_overlap(output_region,required_region) { return Err(ViemStatus::InvalidArgument); }
        let request = unsafe { request.read() };
        if request.struct_size < VIEM_FORMATTED_UTF8_RANGE_V1_SIZE || request.reserved != 0 { return Err(ViemStatus::InvalidArgument); }
        unsafe { out_required.write(0); }
        let fragment = with_core(handle,|core| {
            validate_formatted_snapshot_identity(request.identity,core.document())?;
            core.document().clipboard_fragment(checked_length(request.utf8_start)?..checked_length(request.utf8_end)?)
                .map_err(|_| ViemStatus::InvalidArgument)
        })?;
        unsafe { copy_clipboard_json_bytes(fragment.json().as_bytes(),output,output_capacity,out_required) }
    })
}

/// Import passive HTML or RTF into a portable styled clipboard fragment.
/// This does not create an editable HTML document or execute document content.
/// # Safety
/// Input and output storage must be valid, aligned where typed, and non-overlapping.
#[no_mangle]
pub unsafe extern "C" fn viem_import_clipboard_json(
    format: u32,
    source: *const u8,
    source_length: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(source, source_length)?;
        let output_region = typed_pointer_region(output, output_capacity)?;
        let required_region = typed_pointer_region(out_required, 1)?;
        if regions_overlap(input_region, output_region)
            || regions_overlap(input_region, required_region)
            || regions_overlap(output_region, required_region)
            || source_length > 64 * 1024 * 1024
        { return Err(ViemStatus::InvalidArgument); }
        unsafe { out_required.write(0); }
        let bytes = unsafe { input_bytes(source, source_length)? };
        let fragment = match format {
            VIEM_CLIPBOARD_FORMAT_HTML => crate::document::ClipboardFragment::from_html_utf8(bytes)
                .map(|value| value.0),
            VIEM_CLIPBOARD_FORMAT_RTF => Document::from_bytes(bytes.to_vec(), Encoding::Utf8, Format::Rtf)
                .and_then(|document| document.clipboard_fragment(0..document.projection().text_tree().byte_len())),
            _ => return Err(ViemStatus::InvalidArgument),
        }.map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe { copy_clipboard_json_bytes(fragment.json().as_bytes(), output, output_capacity, out_required) }
    })
}

/// Copy the immutable fragment captured before an emitted clipboard write.
/// A plain-only write has a zero-byte result.
/// # Safety
/// Output storage must be valid and the output regions may not overlap.
#[no_mangle]
pub unsafe extern "C" fn viem_effect_batch_copy_clipboard_json(
    batch: ViemEffectBatchHandle,
    clipboard_index: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let output_region = typed_pointer_region(output,output_capacity)?;
        let required_region = typed_pointer_region(out_required,1)?;
        if regions_overlap(output_region,required_region) { return Err(ViemStatus::InvalidArgument); }
        unsafe { out_required.write(0); }
        let batch = owned_effect_batch(batch)?;
        let write = batch.clipboard_writes.get(checked_length(clipboard_index)?).ok_or(ViemStatus::InvalidArgument)?;
        let bytes = write.content().portable_register().and_then(|value| value.clipboard_fragment()).map_or(&[][..], |fragment| fragment.json().as_bytes());
        unsafe { copy_clipboard_json_bytes(bytes,output,output_capacity,out_required) }
    })
}

unsafe fn copy_clipboard_json_bytes(bytes: &[u8], output: *mut u8, capacity: u64, required: *mut u64) -> Result<(),ViemStatus> {
    unsafe { required.write(checked_export_count(bytes.len())?); }
    if checked_length(capacity)? < bytes.len() { return Err(ViemStatus::BufferTooSmall); }
    if !bytes.is_empty() { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(),output,bytes.len()); } }
    Ok(())
}
