#ifndef EVIM_CORE_H
#define EVIM_CORE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EVIM_CORE_ABI_VERSION 3u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1 1u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2 2u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION \
  EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2

typedef uint64_t EvimDocumentHandle;
typedef uint64_t EvimCoreHandle;
/* Owned immutable command-turn effects; zero means no effects. */
typedef uint64_t EvimEffectBatchHandle;
typedef uint64_t EvimViewId;
typedef uint32_t EvimStatus;

#define EVIM_STATUS_OK 0u
#define EVIM_STATUS_INVALID_ARGUMENT 1u
#define EVIM_STATUS_NULL_POINTER 2u
#define EVIM_STATUS_INVALID_HANDLE 3u
#define EVIM_STATUS_STALE_REVISION 4u
#define EVIM_STATUS_INVALID_UTF8 5u
#define EVIM_STATUS_INVALID_ENCODING 6u
#define EVIM_STATUS_INVALID_FORMAT 7u
#define EVIM_STATUS_INVALID_FILE_FORMAT 8u
#define EVIM_STATUS_BUFFER_TOO_SMALL 9u
#define EVIM_STATUS_INVALID_RANGE 10u
#define EVIM_STATUS_NOT_GRAPHEME_BOUNDARY 11u
#define EVIM_STATUS_UNREPRESENTABLE_CHARACTER 12u
#define EVIM_STATUS_AMBIGUOUS_PROJECTION 13u
#define EVIM_STATUS_UNSUPPORTED_OPERATION 14u
#define EVIM_STATUS_POLICY_REQUIRED 15u
#define EVIM_STATUS_RESOURCE_EXHAUSTED 16u
#define EVIM_STATUS_VERIFICATION_FAILED 17u
#define EVIM_STATUS_LENGTH_OVERFLOW 18u
#define EVIM_STATUS_DOCUMENT_BUSY 19u
#define EVIM_STATUS_CORE_BUSY 20u
#define EVIM_STATUS_INVALID_VIEW 21u
#define EVIM_STATUS_INVALID_PROVIDER 22u
#define EVIM_STATUS_PROVIDER_FAILURE 23u
#define EVIM_STATUS_INVALID_KEY 24u
#define EVIM_STATUS_CORE_FAILURE 25u
#define EVIM_STATUS_UNSTABLE_SHAPING_CONTEXT 26u
#define EVIM_STATUS_VERTICAL_VIEWPORT_ORIGIN_UNSUPPORTED 27u
#define EVIM_STATUS_LAYOUT_UNAVAILABLE 28u
#define EVIM_STATUS_OUTSIDE_LAYOUT_COVERAGE 29u
#define EVIM_STATUS_UNKNOWN_STYLE 30u
#define EVIM_STATUS_STYLE_READ_ONLY 31u
#define EVIM_STATUS_INVALID_STYLE_VALUE 32u
#define EVIM_STATUS_STYLE_INHERITANCE_CYCLE 33u
#define EVIM_STATUS_INCOMPATIBLE_STYLE_ROLE 34u
#define EVIM_STATUS_INVALID_STYLE_RELATIONSHIP 35u
#define EVIM_STATUS_INVALID_UTF8_BOUNDARY 36u
#define EVIM_STATUS_INVALID_UTF16_BOUNDARY 37u
#define EVIM_STATUS_STYLE_EDIT_GROUP_ACTIVE 38u
#define EVIM_STATUS_INVALID_STYLE_EDIT_GROUP 39u
#define EVIM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER 40u
#define EVIM_STATUS_INTERNAL_ERROR 254u
#define EVIM_STATUS_PANIC 255u

/*
 * Core-owned initial detection: supported BOM first, otherwise wholly valid
 * UTF-8, otherwise ISO-8859-1. Nonzero values force the named encoding.
 */
#define EVIM_ENCODING_DETECT 0u
#define EVIM_ENCODING_UTF8 1u
#define EVIM_ENCODING_LATIN1 2u
#define EVIM_ENCODING_UTF16_LE 3u
#define EVIM_ENCODING_UTF16_BE 4u

#define EVIM_FORMAT_PLAIN_TEXT 1u
#define EVIM_FORMAT_MARKDOWN 2u

#define EVIM_FILE_FORMAT_DETECT 0u
#define EVIM_FILE_FORMAT_UNIX 1u
#define EVIM_FILE_FORMAT_DOS 2u
#define EVIM_FILE_FORMAT_MAC 3u

#define EVIM_FILE_FORMAT_ORIGIN_DETECTED 1u
#define EVIM_FILE_FORMAT_ORIGIN_FORCED 2u
#define EVIM_FILE_FORMAT_ORIGIN_DEFAULTED 3u

#define EVIM_HISTORY_ACTION_CATEGORY_NONE 0u
#define EVIM_HISTORY_ACTION_CATEGORY_TEXT 1u
#define EVIM_HISTORY_ACTION_CATEGORY_STYLE 2u
#define EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT 3u
#define EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER 4u
#define EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_SOURCE_RESTORATION 5u
#define EVIM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA 6u
#define EVIM_HISTORY_ACTION_CATEGORY_MIXED 7u

#define EVIM_DOCUMENT_STATE_HAS_BOM (1u << 0)
#define EVIM_DOCUMENT_STATE_CAN_UNDO (1u << 1)
#define EVIM_DOCUMENT_STATE_CAN_REDO (1u << 2)
#define EVIM_DOCUMENT_STATE_IS_DIRTY (1u << 3)

typedef struct EvimDocumentOptions {
  uint32_t struct_size;
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
} EvimDocumentOptions;

#define EVIM_DOCUMENT_OPTIONS_SIZE 16u

/* Model, pipeline, and history metadata captured in one serial core query. */
typedef struct EvimDocumentStateV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
  uint32_t file_format_origin;
  uint32_t undo_action_category;
  uint32_t redo_action_category;
  uint32_t reserved[2];
} EvimDocumentStateV1;

#define EVIM_DOCUMENT_STATE_V1_SIZE \
  ((uint32_t)sizeof(EvimDocumentStateV1))

/* Exact identity of one immutable formatted projection. */
typedef struct EvimFormattedSnapshotIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
} EvimFormattedSnapshotIdentityV1;

#define EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimFormattedSnapshotIdentityV1))

/* Constant-time aggregate metadata; querying it never flattens document text. */
typedef struct EvimFormattedSnapshotInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t utf16_length;
  uint64_t hard_line_count;
} EvimFormattedSnapshotInfoV1;

#define EVIM_FORMATTED_SNAPSHOT_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimFormattedSnapshotInfoV1))

/* Scalar-aligned, half-open UTF-8 range in one exact formatted snapshot. */
typedef struct EvimFormattedUtf8RangeV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_start;
  uint64_t utf8_end;
} EvimFormattedUtf8RangeV1;

#define EVIM_FORMATTED_UTF8_RANGE_V1_SIZE \
  ((uint32_t)sizeof(EvimFormattedUtf8RangeV1))

/*
 * Logical status metadata for one grapheme-aligned formatted point. Line and
 * column values are zero based. hard_line_start..hard_line_end excludes the
 * following semantic hard break.
 */
typedef struct EvimFormattedPointInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_offset;
  uint64_t utf16_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
} EvimFormattedPointInfoV1;

#define EVIM_FORMATTED_POINT_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimFormattedPointInfoV1))

#define EVIM_PROVIDER_THREADING_ANY_WORKER 1u
#define EVIM_PROVIDER_THREADING_DEDICATED_SERIAL 2u
#define EVIM_PROVIDER_THREADING_FRONTEND_MAIN 3u

#define EVIM_RENDER_THREADING_ANY 1u
#define EVIM_RENDER_THREADING_DEDICATED_SERIAL 2u
#define EVIM_RENDER_THREADING_FRONTEND_MAIN 3u

#define EVIM_LAYOUT_EXECUTION_WORKER_POOL 1u
#define EVIM_LAYOUT_EXECUTION_DEDICATED_SERIAL 2u
#define EVIM_LAYOUT_EXECUTION_FRONTEND_MAIN 3u

#define EVIM_TEXT_DIRECTION_AUTO 0u
#define EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT 1u
#define EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT 2u

#define EVIM_FONT_SLANT_UPRIGHT 0u
#define EVIM_FONT_SLANT_ITALIC 1u
#define EVIM_FONT_SLANT_OBLIQUE 2u

#define EVIM_SHAPE_PURPOSE_METRICS_ONLY 1u
#define EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA 2u

#define EVIM_BOUNDARY_AFFINITY_UPSTREAM 1u
#define EVIM_BOUNDARY_AFFINITY_DOWNSTREAM 2u

#define EVIM_KEY_CHARACTER 1u
#define EVIM_KEY_ESCAPE 2u
#define EVIM_KEY_ENTER 3u
#define EVIM_KEY_TAB 4u
#define EVIM_KEY_BACKSPACE 5u
#define EVIM_KEY_DELETE 6u
#define EVIM_KEY_LEFT 7u
#define EVIM_KEY_RIGHT 8u
#define EVIM_KEY_UP 9u
#define EVIM_KEY_DOWN 10u
#define EVIM_KEY_HOME 11u
#define EVIM_KEY_END 12u
#define EVIM_KEY_PAGE_UP 13u
#define EVIM_KEY_PAGE_DOWN 14u
#define EVIM_KEY_CONTROL_CHARACTER 15u

#define EVIM_COMMAND_STATUS_NONE 0u
#define EVIM_COMMAND_STATUS_COMPLETE 1u
#define EVIM_COMMAND_STATUS_PENDING 2u
#define EVIM_COMMAND_STATUS_CANCELLED 3u
#define EVIM_COMMAND_STATUS_NEEDS_MORE_LAYOUT 4u
#define EVIM_COMMAND_STATUS_SEARCH_NOT_FOUND 5u
#define EVIM_COMMAND_STATUS_UNSUPPORTED 6u
#define EVIM_COMMAND_STATUS_ERROR 7u

#define EVIM_MODE_NORMAL 1u
#define EVIM_MODE_INSERT 2u
#define EVIM_MODE_REPLACE 3u
#define EVIM_MODE_VISUAL_CHARACTER 4u
#define EVIM_MODE_VISUAL_LINE 5u
#define EVIM_MODE_VISUAL_BLOCK 6u
#define EVIM_MODE_COMMAND_LINE 7u

#define EVIM_CLIPBOARD_TARGET_CLIPBOARD 1u
#define EVIM_CLIPBOARD_TARGET_PRIMARY 2u

#define EVIM_CLIPBOARD_TURN_HAS_READ (1u << 0)
#define EVIM_CLIPBOARD_TURN_WRITABLE (1u << 1)

#define EVIM_EFFECT_BATCH_HAS_EX_OUTCOME (1u << 0)
#define EVIM_EFFECT_BATCH_EX_DOCUMENT_CHANGED (1u << 1)
#define EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION (1u << 2)
#define EVIM_EFFECT_BATCH_EX_NAVIGATION_HISTORY (1u << 3)

#define EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER (1u << 0)
#define EVIM_REGISTER_KIND_NONE 0u
#define EVIM_REGISTER_KIND_CHARACTER 1u
#define EVIM_REGISTER_KIND_LINE 2u
#define EVIM_REGISTER_KIND_BLOCK 3u

#define EVIM_EX_FRONTEND_EDIT 1u
#define EVIM_EX_FRONTEND_NEW 2u
#define EVIM_EX_FRONTEND_WRITE 3u
#define EVIM_EX_FRONTEND_SAVE_AS 4u
#define EVIM_EX_FRONTEND_QUIT 5u
#define EVIM_EX_FRONTEND_QUIT_ALL 6u
#define EVIM_EX_FRONTEND_WRITE_QUIT 7u
#define EVIM_EX_FRONTEND_XIT 8u
#define EVIM_EX_FRONTEND_WRITE_ALL 9u
#define EVIM_EX_FRONTEND_MARKS 10u
#define EVIM_EX_FRONTEND_REGISTERS 11u
#define EVIM_EX_FRONTEND_JUMPS 12u
#define EVIM_EX_FRONTEND_OPTIONS 13u
#define EVIM_EX_FRONTEND_PRINT_LINES 14u
#define EVIM_EX_FRONTEND_NORMAL 15u

#define EVIM_EX_FRONTEND_FORCE (1u << 0)
#define EVIM_EX_FRONTEND_HAS_PATH (1u << 1)
#define EVIM_EX_FRONTEND_HAS_RANGE (1u << 2)
#define EVIM_EX_FRONTEND_NUMBER (1u << 3)
#define EVIM_EX_FRONTEND_LIST (1u << 4)
#define EVIM_EX_FRONTEND_LITERAL (1u << 5)

#define EVIM_EX_OPTION_WRAP 1u
#define EVIM_EX_OPTION_LINEBREAK 2u
#define EVIM_EX_OPTION_FILE_FORMAT 3u
#define EVIM_EX_OPTION_FILE_FORMATS 4u

#define EVIM_EX_OPTION_VALUE_BOOLEAN 1u
#define EVIM_EX_OPTION_VALUE_FILE_FORMAT 2u
#define EVIM_EX_OPTION_VALUE_FILE_FORMATS 3u

#define EVIM_EX_JUMP_CURRENT (1u << 0)

#define EVIM_OUTCOME_HAS_COMMAND (1u << 0)
#define EVIM_OUTCOME_CURSOR_MOVED (1u << 1)
#define EVIM_OUTCOME_DOCUMENT_CHANGED (1u << 2)
#define EVIM_OUTCOME_MODE_CHANGED (1u << 3)
#define EVIM_OUTCOME_LAYOUT_CHANGED (1u << 4)
#define EVIM_OUTCOME_HAS_POSITION_MAP (1u << 5)
#define EVIM_OUTCOME_HAS_LAYOUT (1u << 6)
#define EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS (1u << 7)
#define EVIM_OUTCOME_HAS_COMPOSITION_CHANGES (1u << 8)

typedef struct EvimUtf8Slice {
  const uint8_t *data;
  uint64_t length;
} EvimUtf8Slice;

/* One target snapshot/capability captured immediately before one input turn. */
typedef struct EvimClipboardTurnEntryV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t target;
  uint32_t reserved;
  uint64_t generation;
  EvimUtf8Slice plain_text;
} EvimClipboardTurnEntryV1;

#define EVIM_CLIPBOARD_TURN_ENTRY_V1_SIZE \
  ((uint32_t)sizeof(EvimClipboardTurnEntryV1))

/* Clipboard and Primary may each occur at most once. */
typedef struct EvimCommandTurnContextV1 {
  uint32_t struct_size;
  uint32_t reserved;
  const EvimClipboardTurnEntryV1 *clipboards;
  uint64_t clipboard_count;
} EvimCommandTurnContextV1;

#define EVIM_COMMAND_TURN_CONTEXT_V1_SIZE \
  ((uint32_t)sizeof(EvimCommandTurnContextV1))

/* Offset and length in the UTF-8 string arena copied with an effect batch. */
typedef struct EvimEffectBytesRefV1 {
  uint64_t offset;
  uint64_t length;
} EvimEffectBytesRefV1;

typedef struct EvimClipboardWriteV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t target;
  uint32_t register_kind;
  uint64_t document_id;
  uint64_t document_revision;
  EvimEffectBytesRefV1 plain_text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} EvimClipboardWriteV1;

#define EVIM_CLIPBOARD_WRITE_V1_SIZE \
  ((uint32_t)sizeof(EvimClipboardWriteV1))

/* Subordinate display value for one EVIM_EX_FRONTEND_OPTIONS request. */
typedef struct EvimExOptionDisplayV1 {
  uint32_t struct_size;
  uint32_t name;
  uint32_t value_kind;
  uint32_t scalar_value;
  uint64_t first_file_format;
  uint64_t file_format_count;
} EvimExOptionDisplayV1;

#define EVIM_EX_OPTION_DISPLAY_V1_SIZE \
  ((uint32_t)sizeof(EvimExOptionDisplayV1))

/* Exact resolved mark plus its captured hard-line text. */
typedef struct EvimExMarkV1 {
  uint32_t struct_size;
  uint32_t name;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  EvimEffectBytesRefV1 line_text;
} EvimExMarkV1;

#define EVIM_EX_MARK_V1_SIZE ((uint32_t)sizeof(EvimExMarkV1))

/* Exact resolved register contents and optional semantic hard-break offsets. */
typedef struct EvimExRegisterV1 {
  uint32_t struct_size;
  uint32_t name;
  uint32_t register_kind;
  uint32_t reserved;
  EvimEffectBytesRefV1 text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} EvimExRegisterV1;

#define EVIM_EX_REGISTER_V1_SIZE ((uint32_t)sizeof(EvimExRegisterV1))

/* Exact jump in oldest-to-newest order plus its captured hard-line text. */
typedef struct EvimExJumpV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t list_index;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  EvimEffectBytesRefV1 line_text;
} EvimExJumpV1;

#define EVIM_EX_JUMP_V1_SIZE ((uint32_t)sizeof(EvimExJumpV1))

/* Exact post-turn formatted hard-line content for PRINT_LINES. */
typedef struct EvimExTextLineV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t hard_line_index;
  uint64_t utf8_start;
  uint64_t utf8_end;
  EvimEffectBytesRefV1 text;
} EvimExTextLineV1;

#define EVIM_EX_TEXT_LINE_V1_SIZE ((uint32_t)sizeof(EvimExTextLineV1))

/*
 * One raw Ex request in execution order. text references the copied UTF-8
 * arena and is an optional path, :normal command string, or concatenated
 * Unicode mark/register-name sequence according to kind. Ranges are inclusive
 * zero-based hard-line indices. Option indices address the option array.
 * first_payload/payload_count address the mark, register, jump, or text-line
 * array selected by the info-request kind.
 */
typedef struct EvimExFrontendRequestV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t flags;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  EvimEffectBytesRefV1 text;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t first_option;
  uint64_t option_count;
  uint64_t first_payload;
  uint64_t payload_count;
} EvimExFrontendRequestV1;

#define EVIM_EX_FRONTEND_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(EvimExFrontendRequestV1))

/* Exact immutable batch identity, flags, and two-pass copy sizes. */
typedef struct EvimEffectBatchInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimEffectBatchHandle batch_handle;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t clipboard_write_count;
  uint64_t ex_request_count;
  uint64_t ex_option_count;
  uint64_t ex_mark_count;
  uint64_t ex_register_count;
  uint64_t ex_jump_count;
  uint64_t ex_text_line_count;
  uint64_t file_format_count;
  uint64_t hard_break_count;
  uint64_t string_bytes;
  uint64_t navigation_utf8_offset;
  uint64_t substitution_count;
} EvimEffectBatchInfoV1;

#define EVIM_EFFECT_BATCH_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimEffectBatchInfoV1))

typedef struct EvimTextMetricsV1 {
  float ascent;
  float descent;
  float leading;
} EvimTextMetricsV1;

typedef struct EvimShapedBoundsV1 {
  float x;
  float y;
  float width;
  float height;
} EvimShapedBoundsV1;

typedef struct EvimOpenTypeFeatureV1 {
  uint8_t tag[4];
  uint32_t value;
} EvimOpenTypeFeatureV1;

typedef struct EvimResolvedTextStyleV1 {
  uint32_t struct_size;
  uint32_t slant;
  uint32_t direction;
  uint32_t has_language;
  uint32_t has_script;
  uint32_t reserved;
  float size;
  float weight;
  float letter_spacing;
  float baseline_shift;
  const EvimUtf8Slice *font_families;
  uint64_t font_family_count;
  EvimUtf8Slice language;
  EvimUtf8Slice script;
  const EvimOpenTypeFeatureV1 *features;
  uint64_t feature_count;
} EvimResolvedTextStyleV1;

#define EVIM_RESOLVED_TEXT_STYLE_V1_SIZE \
  ((uint32_t)sizeof(EvimResolvedTextStyleV1))

typedef struct EvimShapeStyleRunV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  EvimResolvedTextStyleV1 style;
} EvimShapeStyleRunV1;

#define EVIM_SHAPE_STYLE_RUN_V1_SIZE ((uint32_t)sizeof(EvimShapeStyleRunV1))

/*
 * Provider-owned opaque render resource. identifier is an integer token, not
 * a native pointer for core to dereference. The provider keeps the token valid
 * for owner/threading while metrics_generation remains current and the owning
 * view remains attached. It may retire the token when that generation becomes
 * stale or when the view is removed/core is destroyed. ABI v1 has no
 * retain/release transfer; a caller must not retain a stale or detached token.
 */
typedef struct EvimRenderRunHandleV1 {
  uint64_t owner;
  uint64_t identifier;
  uint64_t metrics_generation;
  uint32_t threading;
  uint32_t reserved;
} EvimRenderRunHandleV1;

typedef struct EvimClusterCaretStopV1 {
  uint64_t text_offset;
  float inline_offset;
  uint32_t affinity;
} EvimClusterCaretStopV1;

typedef struct EvimShapedClusterV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  float advance;
  EvimTextMetricsV1 metrics;
  EvimShapedBoundsV1 typographic_bounds;
  EvimShapedBoundsV1 ink_bounds;
  uint32_t bidi_level;
  uint32_t has_render_run;
  EvimUtf8Slice fallback_font;
  const EvimClusterCaretStopV1 *caret_stops;
  uint64_t caret_stop_count;
  EvimRenderRunHandleV1 render_run;
} EvimShapedClusterV1;

#define EVIM_SHAPED_CLUSTER_V1_SIZE ((uint32_t)sizeof(EvimShapedClusterV1))

typedef struct EvimShapingDiagnosticV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  EvimUtf8Slice message;
} EvimShapingDiagnosticV1;

#define EVIM_SHAPING_DIAGNOSTIC_V1_SIZE \
  ((uint32_t)sizeof(EvimShapingDiagnosticV1))

typedef struct EvimShapeRequestV1 {
  uint32_t struct_size;
  uint32_t purpose;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  EvimUtf8Slice text;
  EvimUtf8Slice context_before;
  EvimUtf8Slice context_after;
  const EvimShapeStyleRunV1 *style_runs;
  uint64_t style_run_count;
  EvimResolvedTextStyleV1 default_style;
  float scale;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  /*
   * ABI v2 supplies the containing paragraph's EVIM_TEXT_DIRECTION_* value.
   * It also defines text_start..text_end as a stable ownership interior:
   * shape context_before + text + context_after, then return every whole
   * cluster whose logical start lies in the interior. Such a cluster may end
   * in context_after; omit one whose start lies in context_before. The context
   * slices contain immediately adjacent complete grapheme sequences.
   *
   * ABI v1 providers see zero in this fixed-layout slot and may continue to
   * address it as reserved. Their legacy exact-interior responses remain
   * accepted. The anonymous union preserves source and binary compatibility
   * without changing request-array stride.
   */
  union {
    uint32_t paragraph_base_direction;
    uint32_t reserved;
  };
} EvimShapeRequestV1;

#define EVIM_SHAPE_REQUEST_V1_SIZE ((uint32_t)sizeof(EvimShapeRequestV1))

typedef struct EvimShapeResponseV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  const EvimShapedClusterV1 *clusters;
  uint64_t cluster_count;
  const uint64_t *visual_order;
  uint64_t visual_order_count;
  EvimTextMetricsV1 default_metrics;
  const EvimShapingDiagnosticV1 *diagnostics;
  uint64_t diagnostic_count;
} EvimShapeResponseV1;

/*
 * A response echoes the request's text_start..text_end ownership interior.
 * Under provider ABI v2, returned cluster ends may extend into context_after;
 * response bounds do not expand to include those cluster tails.
 */

#define EVIM_SHAPE_RESPONSE_V1_SIZE ((uint32_t)sizeof(EvimShapeResponseV1))

typedef uint64_t (*EvimMetricsGenerationCallback)(void *context);
typedef uint32_t (*EvimShapeBatchCallback)(
    void *context, const EvimShapeRequestV1 *requests, uint64_t request_count,
    EvimShapeResponseV1 *responses, uint64_t response_capacity);

/*
 * The provider table is copied by evim_core_view_add. Provider ABI v1 and v2
 * are accepted; v1 receives Auto in paragraph_base_direction. Context and
 * callback functions must remain valid until the view is removed or its core is
 * successfully destroyed. EVIM_STATUS_CORE_BUSY means destruction did not
 * occur and does not end these lifetimes. Every response pointer returned by
 * shape_batch must remain readable until the next provider callback for that
 * view; core copies all values immediately. Opaque render-run tokens instead
 * follow the generation/view lifetime documented on EvimRenderRunHandleV1.
 *
 * A successful ABI-v2 response affirms that its ownership interior is stable
 * under arbitrary text outside the supplied bounded context. A provider that
 * cannot make that guarantee returns EVIM_STATUS_UNSTABLE_SHAPING_CONTEXT from
 * shape_batch instead of returning partial or uncacheable measurements. Core
 * then installs and caches nothing from the failed batch.
 */
typedef struct EvimTextMeasurementProviderV1 {
  uint32_t struct_size;
  uint32_t abi_version;
  void *context;
  uint64_t measurement_environment_id;
  uint32_t threading;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  uint32_t reserved;
  EvimMetricsGenerationCallback metrics_generation;
  EvimShapeBatchCallback shape_batch;
} EvimTextMeasurementProviderV1;

#define EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE \
  ((uint32_t)sizeof(EvimTextMeasurementProviderV1))

typedef struct EvimViewOptionsV1 {
  uint32_t struct_size;
  uint32_t execution_context;
  float width;
  float height;
} EvimViewOptionsV1;

#define EVIM_VIEW_OPTIONS_V1_SIZE ((uint32_t)sizeof(EvimViewOptionsV1))

/*
 * Absolute presentation origin. left is always requested. With HAS_TOP, all
 * expected_* fields must match the current immutable layout. Core maps top
 * through its compact exact/estimated height index, lays out only the local
 * viewport plus bounded overscan, and publishes both coordinates atomically.
 * The expected_* identity is ignored for a horizontal-only request.
 */
typedef struct EvimViewportOriginV1 {
  uint32_t struct_size;
  uint32_t flags;
  float left;
  float top;
  uint64_t expected_document_id;
  uint64_t expected_document_revision;
  uint64_t expected_layout_revision;
  uint64_t expected_configuration_generation;
  uint64_t expected_measurement_environment_id;
  uint64_t expected_metrics_generation;
} EvimViewportOriginV1;

#define EVIM_VIEWPORT_ORIGIN_V1_SIZE \
  ((uint32_t)sizeof(EvimViewportOriginV1))
#define EVIM_VIEWPORT_ORIGIN_HAS_TOP (1u << 0)

#define EVIM_VIEWPORT_STATE_WRAP (1u << 0)
#define EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT (1u << 1)
#define EVIM_VIEWPORT_STATE_TOP_EXACT (1u << 2)
#define EVIM_VIEWPORT_STATE_HAS_LAYOUT (1u << 3)
#define EVIM_VIEWPORT_STATE_LINEBREAK (1u << 4)

/*
 * maximum_left is authoritative only with MAXIMUM_LEFT_EXACT. scale is the
 * exact positive view-local magnification used by the current configuration.
 * A missing TOP_EXACT flag means the current top depends on estimated prefix
 * heights; it remains presentation state but is not an exact absolute
 * document y. The dependency identity is captured atomically with these flags
 * and values.
 */
typedef struct EvimViewportStateV1 {
  uint32_t struct_size;
  uint32_t flags;
  float left;
  float top;
  float maximum_left;
  float scale;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} EvimViewportStateV1;

#define EVIM_VIEWPORT_STATE_V1_SIZE \
  ((uint32_t)sizeof(EvimViewportStateV1))

/*
 * Exact identity for one immutable layout snapshot. Copy this value from a
 * snapshot-info query into every dependent geometry request. Core never
 * silently substitutes a newer layout.
 */
typedef struct EvimLayoutSnapshotIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} EvimLayoutSnapshotIdentityV1;

#define EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutSnapshotIdentityV1))

typedef struct EvimLayoutInsetsV1 {
  float top;
  float left;
  float bottom;
  float right;
} EvimLayoutInsetsV1;

typedef struct EvimLayoutRectV1 {
  float x;
  float y;
  float width;
  float height;
} EvimLayoutRectV1;

/* Normalized RGBA components; each value is finite and in [0, 1]. */
typedef struct EvimRgbaV1 {
  float red;
  float green;
  float blue;
  float alpha;
} EvimRgbaV1;

#define EVIM_STYLE_NAMESPACE_BLOCK 1u
#define EVIM_STYLE_NAMESPACE_CHARACTER 2u

#define EVIM_STYLE_ROLE_NONE 0u
#define EVIM_STYLE_ROLE_DOCUMENT 1u
#define EVIM_STYLE_ROLE_PARAGRAPH 2u

#define EVIM_STYLE_ORIGIN_SOURCE_BACKED 1u
#define EVIM_STYLE_ORIGIN_GENERATED_CONFIGURATION 2u
#define EVIM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY 3u

#define EVIM_STYLE_DEFINITION_HAS_PARENT (1u << 0)
#define EVIM_STYLE_DEFINITION_HAS_NEXT_STYLE (1u << 1)
#define EVIM_STYLE_DEFINITION_BASE_DOCUMENT (1u << 2)
#define EVIM_STYLE_DEFINITION_BASE_PARAGRAPH (1u << 3)
#define EVIM_STYLE_DEFINITION_BASE_CHARACTER (1u << 4)

#define EVIM_STYLE_CAPABILITY_EDIT_DECLARATIONS (1u << 0)
#define EVIM_STYLE_CAPABILITY_EDIT_PARENT (1u << 1)
#define EVIM_STYLE_CAPABILITY_EDIT_NEXT_STYLE (1u << 2)
#define EVIM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME (1u << 3)

#define EVIM_STYLE_PROPERTY_CANVAS_BACKGROUND 1u
#define EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP 2u
#define EVIM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT 3u
#define EVIM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM 4u
#define EVIM_STYLE_PROPERTY_CANVAS_PADDING_LEFT 5u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE 6u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_AFTER 7u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING 8u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT 9u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT 10u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT 11u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT 12u
#define EVIM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION 13u
#define EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES 14u
#define EVIM_STYLE_PROPERTY_CHARACTER_SIZE 15u
#define EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT 16u
#define EVIM_STYLE_PROPERTY_CHARACTER_SLANT 17u
#define EVIM_STYLE_PROPERTY_CHARACTER_FOREGROUND 18u
#define EVIM_STYLE_PROPERTY_CHARACTER_BACKGROUND 19u
#define EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE 20u
#define EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH 21u
#define EVIM_STYLE_PROPERTY_CHARACTER_LANGUAGE 22u
#define EVIM_STYLE_PROPERTY_CHARACTER_DIRECTION 23u
#define EVIM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES 24u
#define EVIM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING 25u
#define EVIM_STYLE_PROPERTY_CHARACTER_BASELINE_SHIFT 26u

#define EVIM_STYLE_VALUE_NONE 0u
#define EVIM_STYLE_VALUE_FLOAT 1u
#define EVIM_STYLE_VALUE_UNSIGNED 2u
#define EVIM_STYLE_VALUE_BOOLEAN 3u
#define EVIM_STYLE_VALUE_COLOR 4u
#define EVIM_STYLE_VALUE_STRING 5u
#define EVIM_STYLE_VALUE_STRING_LIST 6u
#define EVIM_STYLE_VALUE_FONT_SLANT 7u
#define EVIM_STYLE_VALUE_WRITING_DIRECTION 8u
#define EVIM_STYLE_VALUE_OPEN_TYPE_FEATURES 9u
#define EVIM_STYLE_VALUE_LINE_SPACING 10u
#define EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT 11u

#define EVIM_STYLE_VALUE_ITEM_STRING 1u
#define EVIM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE 2u

#define EVIM_STYLE_LINE_SPACING_NORMAL 1u
#define EVIM_STYLE_LINE_SPACING_MULTIPLIER 2u
#define EVIM_STYLE_LINE_SPACING_AT_LEAST 3u
#define EVIM_STYLE_LINE_SPACING_EXACT 4u

#define EVIM_STYLE_PARAGRAPH_ALIGNMENT_START 1u
#define EVIM_STYLE_PARAGRAPH_ALIGNMENT_END 2u
#define EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER 3u

#define EVIM_STYLE_PROPERTY_DECLARED (1u << 0)
#define EVIM_STYLE_PROPERTY_EFFECTIVE_PRESENT (1u << 1)
#define EVIM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE (1u << 2)

#define EVIM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY 1u
#define EVIM_STYLE_CONTRIBUTOR_BLOCK_STYLE 2u
#define EVIM_STYLE_CONTRIBUTOR_CHARACTER_STYLE 3u
#define EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS 4u
#define EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER 5u
#define EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH 6u
#define EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER 7u
#define EVIM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER 8u

#define EVIM_STYLE_EDIT_SET_DECLARATION 1u
#define EVIM_STYLE_EDIT_CLEAR_DECLARATION 2u
#define EVIM_STYLE_EDIT_SET_PARENT 3u
#define EVIM_STYLE_EDIT_CLEAR_PARENT 4u
#define EVIM_STYLE_EDIT_SET_NEXT_STYLE 5u
#define EVIM_STYLE_EDIT_CLEAR_NEXT_STYLE 6u
#define EVIM_STYLE_EDIT_SET_DISPLAY_NAME 7u

typedef struct EvimStyleSheetIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
} EvimStyleSheetIdentityV1;

#define EVIM_STYLE_SHEET_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleSheetIdentityV1))

typedef struct EvimStyleStringRefV1 {
  uint64_t offset;
  uint64_t length;
} EvimStyleStringRefV1;

typedef struct EvimStyleSheetInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimStyleSheetIdentityV1 identity;
  uint64_t definition_count;
  uint64_t property_count;
  uint64_t value_item_count;
  uint64_t dependency_count;
  uint64_t string_bytes;
} EvimStyleSheetInfoV1;

#define EVIM_STYLE_SHEET_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleSheetInfoV1))

typedef struct EvimStyleValueV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t enum_value;
  uint32_t reserved;
  float number;
  float number_reserved;
  EvimRgbaV1 color;
  EvimStyleStringRefV1 string;
  uint64_t first_item;
  uint64_t item_count;
} EvimStyleValueV1;

#define EVIM_STYLE_VALUE_V1_SIZE ((uint32_t)sizeof(EvimStyleValueV1))

typedef struct EvimStyleValueItemV1 {
  uint32_t struct_size;
  uint32_t kind;
  EvimStyleStringRefV1 string;
  uint32_t unsigned_value;
  uint32_t reserved;
} EvimStyleValueItemV1;

#define EVIM_STYLE_VALUE_ITEM_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleValueItemV1))

typedef struct EvimStyleDependencyV1 {
  uint32_t struct_size;
  uint32_t namespace_id;
  EvimStyleStringRefV1 style_id;
} EvimStyleDependencyV1;

#define EVIM_STYLE_DEPENDENCY_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleDependencyV1))

typedef struct EvimStyleDefinitionV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t namespace_id;
  uint32_t role;
  uint32_t origin;
  uint32_t capabilities;
  EvimStyleStringRefV1 stable_id;
  EvimStyleStringRefV1 display_name;
  EvimStyleStringRefV1 parent_id;
  EvimStyleStringRefV1 next_style_id;
  uint64_t first_property;
  uint64_t property_count;
} EvimStyleDefinitionV1;

#define EVIM_STYLE_DEFINITION_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleDefinitionV1))

typedef struct EvimStylePropertyV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t property;
  uint32_t contributor_kind;
  uint32_t contributor_namespace;
  uint32_t reserved;
  EvimStyleValueV1 declared;
  EvimStyleValueV1 effective;
  EvimStyleStringRefV1 contributor_style_id;
  uint64_t first_dependency;
  uint64_t dependency_count;
} EvimStylePropertyV1;

#define EVIM_STYLE_PROPERTY_V1_SIZE \
  ((uint32_t)sizeof(EvimStylePropertyV1))

typedef struct EvimStyleEditValueItemV1 {
  uint32_t struct_size;
  uint32_t kind;
  EvimUtf8Slice text;
  uint32_t unsigned_value;
  uint32_t reserved;
} EvimStyleEditValueItemV1;

#define EVIM_STYLE_EDIT_VALUE_ITEM_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleEditValueItemV1))

typedef struct EvimStyleEditValueV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t enum_value;
  uint32_t reserved;
  float number;
  float number_reserved;
  EvimRgbaV1 color;
  EvimUtf8Slice text;
  const EvimStyleEditValueItemV1 *items;
  uint64_t item_count;
} EvimStyleEditValueV1;

#define EVIM_STYLE_EDIT_VALUE_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleEditValueV1))

typedef struct EvimStyleEditV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimStyleSheetIdentityV1 identity;
  uint32_t namespace_id;
  uint32_t operation;
  uint32_t property;
  uint32_t reserved;
  EvimUtf8Slice style_id;
  EvimStyleEditValueV1 value;
} EvimStyleEditV1;

#define EVIM_STYLE_EDIT_V1_SIZE ((uint32_t)sizeof(EvimStyleEditV1))

/*
 * Immutable capability for one explicit live style-edit group. token is
 * process-wide and non-reused. Every other field is also part of the
 * capability and must be returned unchanged. Begin revisions remain fixed as
 * grouped edits commit newer snapshots.
 */
typedef struct EvimStyleEditGroupV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t token;
  EvimViewId view_id;
  uint64_t document_id;
  uint64_t begin_document_revision;
  uint64_t begin_style_sheet_revision;
} EvimStyleEditGroupV1;

#define EVIM_STYLE_EDIT_GROUP_V1_SIZE \
  ((uint32_t)sizeof(EvimStyleEditGroupV1))

#define EVIM_TEXT_PAINT_HAS_BACKGROUND (1u << 0)
#define EVIM_TEXT_PAINT_UNDERLINE (1u << 1)
#define EVIM_TEXT_PAINT_STRIKETHROUGH (1u << 2)

/*
 * Foreground is always present. Background is meaningful only with
 * HAS_BACKGROUND. Decoration flags mean that decoration is enabled.
 */
typedef struct EvimTextPaintV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimRgbaV1 foreground;
  EvimRgbaV1 background;
} EvimTextPaintV1;

#define EVIM_TEXT_PAINT_V1_SIZE ((uint32_t)sizeof(EvimTextPaintV1))

/* Canvas/default paint and required override-run count for one exact layout. */
typedef struct EvimLayoutPaintInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimLayoutSnapshotIdentityV1 identity;
  EvimRgbaV1 canvas_background;
  EvimTextPaintV1 default_paint;
  uint64_t paint_run_count;
} EvimLayoutPaintInfoV1;

#define EVIM_LAYOUT_PAINT_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutPaintInfoV1))

/* Ordered, non-overlapping logical half-open UTF-8 override range. */
typedef struct EvimPaintStyleRunV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  EvimTextPaintV1 paint;
} EvimPaintStyleRunV1;

#define EVIM_PAINT_STYLE_RUN_V1_SIZE \
  ((uint32_t)sizeof(EvimPaintStyleRunV1))

#define EVIM_LAYOUT_SNAPSHOT_FULL_DOCUMENT (1u << 0)
#define EVIM_LAYOUT_SNAPSHOT_PREFIX_EXACT (1u << 1)
#define EVIM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT (1u << 2)
#define EVIM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT (1u << 3)

/* Geometry is in document-layout coordinates; subtract the viewport origin. */
typedef struct EvimLayoutSnapshotInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimLayoutSnapshotIdentityV1 identity;
  float viewport_width;
  float viewport_height;
  float usable_width;
  float content_width;
  float total_height;
  EvimLayoutInsetsV1 content_insets;
  uint64_t coverage_hard_line_start;
  uint64_t coverage_hard_line_end;
  uint64_t document_hard_line_count;
  float coverage_y_start;
  float coverage_y_end;
  uint64_t row_count;
  uint64_t cluster_count;
  uint64_t caret_count;
} EvimLayoutSnapshotInfoV1;

#define EVIM_LAYOUT_SNAPSHOT_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutSnapshotInfoV1))

#define EVIM_VISUAL_ROW_HAS_PARAGRAPH (1u << 0)
#define EVIM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS (1u << 1)
#define EVIM_VISUAL_ROW_WRAPS_TO_NEXT (1u << 2)

/* first_* and *_count index arrays copied by the same snapshot export. */
typedef struct EvimVisualRowV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t row_index;
  uint64_t paragraph_id;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t text_start;
  uint64_t text_end;
  float y;
  float baseline;
  float ascent;
  float descent;
  float leading;
  float line_advance;
  float width;
  float paragraph_content_x;
  float paragraph_content_width;
  uint64_t first_cluster;
  uint64_t cluster_count;
  uint64_t first_caret;
  uint64_t caret_count;
} EvimVisualRowV1;

#define EVIM_VISUAL_ROW_V1_SIZE ((uint32_t)sizeof(EvimVisualRowV1))

#define EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN (1u << 0)

typedef struct EvimPositionedClusterV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t row_index;
  uint64_t text_start;
  uint64_t text_end;
  float x;
  float advance;
  EvimLayoutRectV1 typographic_bounds;
  EvimLayoutRectV1 ink_bounds;
  uint32_t bidi_level;
  uint32_t reserved;
  EvimRenderRunHandleV1 render_run;
} EvimPositionedClusterV1;

#define EVIM_POSITIONED_CLUSTER_V1_SIZE \
  ((uint32_t)sizeof(EvimPositionedClusterV1))

typedef struct EvimPositionedCaretV1 {
  uint32_t struct_size;
  uint32_t affinity;
  uint64_t row_index;
  uint64_t text_offset;
  float x;
  float reserved;
} EvimPositionedCaretV1;

#define EVIM_POSITIONED_CARET_V1_SIZE \
  ((uint32_t)sizeof(EvimPositionedCaretV1))

typedef struct EvimLayoutCaretRequestV1 {
  uint32_t struct_size;
  uint32_t affinity;
  EvimLayoutSnapshotIdentityV1 identity;
  uint64_t text_offset;
} EvimLayoutCaretRequestV1;

#define EVIM_LAYOUT_CARET_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutCaretRequestV1))

typedef struct EvimLayoutHitTestRequestV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimLayoutSnapshotIdentityV1 identity;
  float x;
  float y;
} EvimLayoutHitTestRequestV1;

#define EVIM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutHitTestRequestV1))

typedef struct EvimLayoutCaretPointV1 {
  uint32_t struct_size;
  uint32_t affinity;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t text_offset;
} EvimLayoutCaretPointV1;

#define EVIM_LAYOUT_CARET_POINT_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutCaretPointV1))

#define EVIM_CARET_GEOMETRY_CLUSTER_FALLBACK (1u << 0)

typedef struct EvimLayoutCaretGeometryV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimLayoutCaretPointV1 point;
  EvimLayoutRectV1 rect;
  uint64_t row_index;
} EvimLayoutCaretGeometryV1;

#define EVIM_LAYOUT_CARET_GEOMETRY_V1_SIZE \
  ((uint32_t)sizeof(EvimLayoutCaretGeometryV1))

#define EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR (1u << 0)
#define EVIM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT (1u << 1)
#define EVIM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK (1u << 2)
#define EVIM_VIEW_PRESENTATION_HAS_COMMAND_LINE (1u << 3)
#define EVIM_VIEW_PRESENTATION_HAS_DESIRED_X (1u << 4)

/*
 * Linear Visual anchors have no retained visual affinity, so the anchor
 * affinity is zero unless VISUAL_ANCHOR_AFFINITY_EXACT is present. Visual
 * Block endpoints retain exact affinity and display-space x edges.
 */
typedef struct EvimViewPresentationV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t mode;
  uint32_t cursor_affinity;
  uint32_t visual_anchor_affinity;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t cursor_utf8_offset;
  uint64_t visual_anchor_utf8_offset;
  float visual_block_left_x;
  float visual_block_right_x;
  float desired_x;
  float reserved_float;
  uint64_t command_line_utf8_length;
  uint64_t command_line_cursor_utf8_offset;
} EvimViewPresentationV1;

#define EVIM_VIEW_PRESENTATION_V1_SIZE \
  ((uint32_t)sizeof(EvimViewPresentationV1))

#define EVIM_COMMAND_LINE_KIND_NONE 0u
#define EVIM_COMMAND_LINE_KIND_EX 1u
#define EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD 2u
#define EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD 3u

/*
 * state_identity is an opaque exact-content token. It changes whenever the
 * exported kind, UTF-8 cursor, or bytes change without a document revision.
 */
typedef struct EvimCommandLineIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint8_t state_identity[32];
} EvimCommandLineIdentityV1;

#define EVIM_COMMAND_LINE_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimCommandLineIdentityV1))

/* The prompt prefix is represented by identity.kind, not by the UTF-8 bytes. */
typedef struct EvimCommandLineInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimCommandLineIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t cursor_utf8_offset;
} EvimCommandLineInfoV1;

#define EVIM_COMMAND_LINE_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimCommandLineInfoV1))

#define EVIM_VISUAL_SELECTION_KIND_NONE 0u
#define EVIM_VISUAL_SELECTION_KIND_CHARACTER 1u
#define EVIM_VISUAL_SELECTION_KIND_LINE 2u
#define EVIM_VISUAL_SELECTION_KIND_BLOCK 3u

#define EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW (1u << 0)
#define EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE (1u << 1)
#define EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES (1u << 2)

/*
 * layout identifies the exact immutable geometry. state_identity is an opaque
 * exact-payload token that also changes when selection state changes without
 * relayout.
 */
typedef struct EvimVisualSelectionIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  EvimLayoutSnapshotIdentityV1 layout;
  uint8_t state_identity[32];
} EvimVisualSelectionIdentityV1;

#define EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimVisualSelectionIdentityV1))

/*
 * Segments are in logical UTF-8 document order. Rectangles are in visual-row,
 * then x order.
 */
typedef struct EvimVisualSelectionInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimVisualSelectionIdentityV1 identity;
  uint64_t segment_count;
  uint64_t rectangle_count;
} EvimVisualSelectionInfoV1;

#define EVIM_VISUAL_SELECTION_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimVisualSelectionInfoV1))

/*
 * One logical half-open UTF-8 range. Character/Line segments have zero flags.
 * Block segments retain their visual row, hard line, and display-left/right
 * affinities; these affinities are not logical start/end labels in bidi text.
 */
typedef struct EvimVisualSelectionSegmentV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t text_start;
  uint64_t text_end;
  uint64_t row_index;
  uint64_t hard_line_index;
  uint32_t left_affinity;
  uint32_t right_affinity;
} EvimVisualSelectionSegmentV1;

#define EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE \
  ((uint32_t)sizeof(EvimVisualSelectionSegmentV1))

/* segment_index indexes the segment array copied by the same call. */
typedef struct EvimVisualSelectionRectangleV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t row_index;
  uint64_t segment_index;
  EvimLayoutRectV1 rect;
} EvimVisualSelectionRectangleV1;

#define EVIM_VISUAL_SELECTION_RECTANGLE_V1_SIZE \
  ((uint32_t)sizeof(EvimVisualSelectionRectangleV1))

#define EVIM_LOGICAL_SELECTION_KIND_NONE 0u
#define EVIM_LOGICAL_SELECTION_KIND_CHARACTER 1u
#define EVIM_LOGICAL_SELECTION_KIND_LINE 2u
#define EVIM_LOGICAL_SELECTION_KIND_BLOCK 3u

#define EVIM_SEMANTIC_STYLE_STRONG 1u
#define EVIM_SEMANTIC_STYLE_EMPHASIS 2u

#define EVIM_SEMANTIC_STYLE_STATE_OFF 0u
#define EVIM_SEMANTIC_STYLE_STATE_ON 1u
#define EVIM_SEMANTIC_STYLE_STATE_MIXED 2u

#define EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE (1u << 0)
#define EVIM_SEMANTIC_STYLE_CAN_SET (1u << 1)
#define EVIM_SEMANTIC_STYLE_CAN_CLEAR (1u << 2)

/*
 * Exact logical selection identity with no layout dependency. Only Character
 * and Line selections name an actionable contiguous range in this ABI.
 */
typedef struct EvimLogicalSelectionIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t text_start;
  uint64_t text_end;
  uint8_t state_identity[32];
} EvimLogicalSelectionIdentityV1;

#define EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimLogicalSelectionIdentityV1))

/* Adapter capability and check/mixed state at one core-owned selection. */
typedef struct EvimSemanticStylePresentationV1 {
  uint32_t struct_size;
  uint32_t style;
  uint32_t state;
  uint32_t flags;
  EvimLogicalSelectionIdentityV1 selection;
} EvimSemanticStylePresentationV1;

#define EVIM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE \
  ((uint32_t)sizeof(EvimSemanticStylePresentationV1))

/* Exact request using an identity returned by the presentation query. */
typedef struct EvimSetSemanticStyleV1 {
  uint32_t struct_size;
  uint32_t style;
  uint32_t enabled;
  uint32_t reserved;
  EvimLogicalSelectionIdentityV1 expected_selection;
} EvimSetSemanticStyleV1;

#define EVIM_SET_SEMANTIC_STYLE_V1_SIZE \
  ((uint32_t)sizeof(EvimSetSemanticStyleV1))

#define EVIM_PLACE_CURSOR_EXTEND_SELECTION (1u << 0)

typedef struct EvimPlaceCursorV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t document_revision;
  uint64_t text_offset;
  uint32_t affinity;
  uint32_t reserved;
} EvimPlaceCursorV1;

#define EVIM_PLACE_CURSOR_V1_SIZE ((uint32_t)sizeof(EvimPlaceCursorV1))

/* Concrete file-format target bound to one exact document snapshot. */
typedef struct EvimSetFileFormatV1 {
  uint32_t struct_size;
  uint32_t file_format;
  uint64_t document_id;
  uint64_t document_revision;
} EvimSetFileFormatV1;

#define EVIM_SET_FILE_FORMAT_V1_SIZE \
  ((uint32_t)sizeof(EvimSetFileFormatV1))

/* Exact acknowledgement of a successfully persisted native snapshot. */
typedef struct EvimMarkSavedV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
} EvimMarkSavedV1;

#define EVIM_MARK_SAVED_V1_SIZE ((uint32_t)sizeof(EvimMarkSavedV1))

typedef struct EvimKeyInputV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t codepoint;
  uint32_t reserved;
} EvimKeyInputV1;

#define EVIM_KEY_INPUT_V1_SIZE ((uint32_t)sizeof(EvimKeyInputV1))

/*
 * Exact formatted-snapshot replacement target for native marked text. The
 * frontend passes its current selection, or an empty range at its insertion
 * caret, using UTF-8 byte offsets for document_revision.
 */
typedef struct EvimCompositionBeginV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  uint64_t replacement_start;
  uint64_t replacement_end;
} EvimCompositionBeginV1;

#define EVIM_COMPOSITION_BEGIN_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionBeginV1))

/* selected_start/end are UTF-8 byte offsets relative to marked_text. */
typedef struct EvimCompositionUpdateV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  EvimUtf8Slice marked_text;
  uint64_t selected_start;
  uint64_t selected_end;
} EvimCompositionUpdateV1;

#define EVIM_COMPOSITION_UPDATE_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionUpdateV1))

typedef struct EvimCompositionCommitV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  EvimUtf8Slice committed_text;
} EvimCompositionCommitV1;

#define EVIM_COMPOSITION_COMMIT_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionCommitV1))

typedef struct EvimCompositionCancelV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
} EvimCompositionCancelV1;

#define EVIM_COMPOSITION_CANCEL_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionCancelV1))

#define EVIM_COMPOSITION_OVERLAY_ACTIVE (1u << 0)

typedef struct EvimCompositionOverlayIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t generation;
} EvimCompositionOverlayIdentityV1;

#define EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionOverlayIdentityV1))

/* Exact ranges in the disposable, source-nonmutating composed projection. */
typedef struct EvimCompositionOverlayInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  EvimCompositionOverlayIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t replacement_start;
  uint64_t replacement_end;
  uint64_t marked_start;
  uint64_t marked_end;
  uint64_t selected_start;
  uint64_t selected_end;
} EvimCompositionOverlayInfoV1;

#define EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionOverlayInfoV1))

typedef struct EvimCompositionOverlayUtf8RangeV1 {
  uint32_t struct_size;
  uint32_t reserved;
  EvimCompositionOverlayIdentityV1 identity;
  uint64_t start;
  uint64_t end;
} EvimCompositionOverlayUtf8RangeV1;

#define EVIM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionOverlayUtf8RangeV1))

typedef struct EvimCoreOutcomeV1 {
  uint32_t struct_size;
  uint32_t command_status;
  uint32_t mode;
  uint32_t flags;
  uint64_t document_revision;
  uint64_t view_id;
  uint64_t cursor_utf8_offset;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} EvimCoreOutcomeV1;

#define EVIM_CORE_OUTCOME_V1_SIZE ((uint32_t)sizeof(EvimCoreOutcomeV1))

uint32_t evim_core_abi_version(void);

/*
 * All strings and byte sequences are length-delimited and are never assumed
 * or written to be NUL-terminated. A null data pointer is permitted only for
 * a zero-length input or a zero-capacity length query.
 *
 * Document handles are opaque, process-local, nonzero tokens. They are never
 * reused. Calls are thread-safe. A document is checked out for one serial
 * operation without retaining a registry lock; a concurrent operation on the
 * same document returns EVIM_STATUS_DOCUMENT_BUSY instead of blocking.
 * Destroying a checked-out document also returns EVIM_STATUS_DOCUMENT_BUSY,
 * retains the handle, and must be retried after the operation returns.
 */
EvimStatus evim_document_create(const uint8_t *source,
                                uint64_t source_length,
                                const EvimDocumentOptions *options,
                                EvimDocumentHandle *out_document,
                                uint64_t *out_revision);

EvimStatus evim_document_destroy(EvimDocumentHandle document);

EvimStatus evim_document_revision(EvimDocumentHandle document,
                                  uint64_t *out_revision);

/*
 * For either copy function, pass output=NULL and output_capacity=0 to query
 * the required byte count. A nonempty value then returns
 * EVIM_STATUS_BUFFER_TOO_SMALL and always populates out_required.
 */
EvimStatus evim_document_copy_source_bytes(EvimDocumentHandle document,
                                           uint64_t expected_revision,
                                           uint8_t *output,
                                           uint64_t output_capacity,
                                           uint64_t *out_required);

EvimStatus evim_document_copy_formatted_utf8(EvimDocumentHandle document,
                                             uint64_t expected_revision,
                                             uint8_t *output,
                                             uint64_t output_capacity,
                                             uint64_t *out_required);

/* start and end are half-open byte offsets in the formatted UTF-8 snapshot. */
EvimStatus evim_document_replace_formatted_utf8(
    EvimDocumentHandle document, uint64_t expected_revision, uint64_t start,
    uint64_t end, const uint8_t *replacement, uint64_t replacement_length,
    uint64_t *out_revision);

/*
 * Core handles own a document, command/controller state, and attached views.
 * They are opaque, process-local, nonzero, and never reused. The core registry
 * lock is never held while invoking a provider callback. Reentrant access to
 * the same checked-out core returns EVIM_STATUS_CORE_BUSY. Destroying a
 * checked-out core does the same without removing it; the caller must retain
 * all provider-owned state and retry destruction after the operation returns.
 */
EvimStatus evim_core_create(const uint8_t *source, uint64_t source_length,
                            const EvimDocumentOptions *options,
                            EvimCoreHandle *out_core,
                            uint64_t *out_revision);

EvimStatus evim_core_destroy(EvimCoreHandle core);
EvimStatus evim_core_revision(EvimCoreHandle core, uint64_t *out_revision);
EvimStatus evim_core_document_state(EvimCoreHandle core,
                                    EvimDocumentStateV1 *out_state);

/*
 * Bounded access to the immutable formatted projection. Snapshot info returns
 * the exact identity used by all dependent operations. The range and mapping
 * copies are all-or-none: a short output returns BUFFER_TOO_SMALL and the
 * complete required byte/element count without writing any output item.
 * Invalid scalar/surrogate boundaries have distinct typed statuses, and an
 * identity mismatch returns STALE_REVISION.
 */
EvimStatus evim_core_formatted_snapshot_info(
    EvimCoreHandle core, EvimFormattedSnapshotInfoV1 *out_info);
EvimStatus evim_core_copy_formatted_utf8_range(
    EvimCoreHandle core, const EvimFormattedUtf8RangeV1 *request,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
EvimStatus evim_core_map_formatted_utf8_to_utf16(
    EvimCoreHandle core, const EvimFormattedSnapshotIdentityV1 *identity,
    const uint64_t *utf8_offsets, uint64_t offset_count,
    uint64_t *utf16_offsets, uint64_t output_capacity,
    uint64_t *out_required);
EvimStatus evim_core_map_formatted_utf16_to_utf8(
    EvimCoreHandle core, const EvimFormattedSnapshotIdentityV1 *identity,
    const uint64_t *utf16_offsets, uint64_t offset_count,
    uint64_t *utf8_offsets, uint64_t output_capacity,
    uint64_t *out_required);
EvimStatus evim_core_formatted_point_info(
    EvimCoreHandle core, const EvimFormattedSnapshotIdentityV1 *identity,
    uint64_t utf8_offset, EvimFormattedPointInfoV1 *out_info);

/* Call only after the native write of this exact source snapshot succeeds. */
EvimStatus evim_core_mark_saved(EvimCoreHandle core,
                                const EvimMarkSavedV1 *request);

/*
 * Query/copy one exact immutable normalized style sheet. The copy is all-or-
 * none across every typed array and the UTF-8 string arena. A zero-capacity
 * call returns BUFFER_TOO_SMALL plus exact current sizes. Any identity change
 * returns STALE_REVISION rather than substituting a newer sheet.
 */
EvimStatus evim_core_style_sheet_info(EvimCoreHandle core,
                                      EvimStyleSheetInfoV1 *out_info);
EvimStatus evim_core_copy_style_sheet(
    EvimCoreHandle core, const EvimStyleSheetIdentityV1 *expected,
    EvimStyleDefinitionV1 *definitions, uint64_t definition_capacity,
    EvimStylePropertyV1 *properties, uint64_t property_capacity,
    EvimStyleValueItemV1 *value_items, uint64_t value_item_capacity,
    EvimStyleDependencyV1 *dependencies, uint64_t dependency_capacity,
    uint8_t *string_bytes, uint64_t string_capacity,
    EvimStyleSheetInfoV1 *out_info);

EvimStatus evim_core_view_add(
    EvimCoreHandle core, const EvimViewOptionsV1 *options,
    const EvimTextMeasurementProviderV1 *provider, EvimViewId *out_view,
    EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_remove(EvimCoreHandle core, EvimViewId view);
EvimStatus evim_core_view_state(EvimCoreHandle core, EvimViewId view,
                                EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_viewport_state(EvimCoreHandle core, EvimViewId view,
                                         EvimViewportStateV1 *out_state);

/*
 * Snapshot info is an atomic current-layout query. To copy geometry, pass its
 * exact identity to copy_layout_snapshot. Null array pointers are accepted
 * only with zero capacities; BUFFER_TOO_SMALL writes required counts to
 * out_info and writes no array elements. Render-run tokens are borrowed under
 * their provider generation/view lifetime and are not retained by this call.
 */
EvimStatus evim_core_view_layout_snapshot_info(
    EvimCoreHandle core, EvimViewId view, EvimLayoutSnapshotInfoV1 *out_info);
EvimStatus evim_core_view_copy_layout_snapshot(
    EvimCoreHandle core, EvimViewId view,
    const EvimLayoutSnapshotIdentityV1 *expected,
    EvimVisualRowV1 *rows, uint64_t row_capacity,
    EvimPositionedClusterV1 *clusters, uint64_t cluster_capacity,
    EvimPositionedCaretV1 *carets, uint64_t caret_capacity,
    EvimLayoutSnapshotInfoV1 *out_info);

/*
 * Paint info is an atomic query against the current immutable layout. Pass its
 * exact identity to copy_layout_paint. Null run output is accepted only with
 * zero capacity; BUFFER_TOO_SMALL writes the required count and fixed paint
 * state to out_info and writes no run elements.
 */
EvimStatus evim_core_view_layout_paint_info(
    EvimCoreHandle core, EvimViewId view, EvimLayoutPaintInfoV1 *out_info);
EvimStatus evim_core_view_copy_layout_paint(
    EvimCoreHandle core, EvimViewId view,
    const EvimLayoutSnapshotIdentityV1 *expected,
    EvimPaintStyleRunV1 *runs, uint64_t run_capacity,
    EvimLayoutPaintInfoV1 *out_info);

EvimStatus evim_core_view_caret_geometry(
    EvimCoreHandle core, EvimViewId view,
    const EvimLayoutCaretRequestV1 *request,
    EvimLayoutCaretGeometryV1 *out_geometry);
EvimStatus evim_core_view_layout_hit_test(
    EvimCoreHandle core, EvimViewId view,
    const EvimLayoutHitTestRequestV1 *request,
    EvimLayoutCaretPointV1 *out_point);
EvimStatus evim_core_view_presentation(
    EvimCoreHandle core, EvimViewId view,
    EvimViewPresentationV1 *out_presentation);

/*
 * Command-line info is an atomic current-state query. Copy requires its exact
 * identity. Null UTF-8 output is accepted only with zero capacity;
 * BUFFER_TOO_SMALL writes the current required length and no bytes.
 */
EvimStatus evim_core_view_command_line_info(
    EvimCoreHandle core, EvimViewId view, EvimCommandLineInfoV1 *out_info);
EvimStatus evim_core_view_copy_command_line(
    EvimCoreHandle core, EvimViewId view,
    const EvimCommandLineIdentityV1 *expected,
    uint8_t *utf8, uint64_t utf8_capacity,
    EvimCommandLineInfoV1 *out_info);

/*
 * Visual-selection info resolves the current selection against the exact
 * current layout. Copy requires both its layout and opaque state identities.
 * Null array pointers are accepted only with zero capacities;
 * BUFFER_TOO_SMALL writes both required counts and no array elements.
 * STALE_REVISION means either identity changed. OUTSIDE_LAYOUT_COVERAGE means
 * the current exact selection is not wholly materialized by this layout.
 */
EvimStatus evim_core_view_visual_selection_info(
    EvimCoreHandle core, EvimViewId view,
    EvimVisualSelectionInfoV1 *out_info);
EvimStatus evim_core_view_copy_visual_selection(
    EvimCoreHandle core, EvimViewId view,
    const EvimVisualSelectionIdentityV1 *expected,
    EvimVisualSelectionSegmentV1 *segments, uint64_t segment_capacity,
    EvimVisualSelectionRectangleV1 *rectangles, uint64_t rectangle_capacity,
    EvimVisualSelectionInfoV1 *out_info);

/*
 * Query source-backed Strong/Emphasis state and capability using only the
 * current core-owned logical selection. No layout snapshot is required.
 */
EvimStatus evim_core_view_semantic_style_presentation(
    EvimCoreHandle core, EvimViewId view, uint32_t style,
    EvimSemanticStylePresentationV1 *out_presentation);

/* Apply to the exact logical-selection identity returned by the query. */
EvimStatus evim_core_view_set_semantic_style(
    EvimCoreHandle core, EvimViewId view,
    const EvimSetSemanticStyleV1 *request,
    EvimCoreOutcomeV1 *out_outcome);

/*
 * Bind an exact current Visual selection as the literal forward-search target.
 * The selection identity includes both layout and opaque controller state;
 * stale requests fail rather than capturing different text.
 */
EvimStatus evim_core_view_use_selection_for_find(
    EvimCoreHandle core, EvimViewId view,
    const EvimVisualSelectionIdentityV1 *expected,
    EvimCoreOutcomeV1 *out_outcome);

/* Reveal the active endpoint of the core-owned Visual selection. */
EvimStatus evim_core_view_reveal_selection(
    EvimCoreHandle core, EvimViewId view,
    EvimCoreOutcomeV1 *out_outcome);

EvimStatus evim_core_view_send_key(EvimCoreHandle core, EvimViewId view,
                                   const EvimKeyInputV1 *input,
                                   EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_send_text(EvimCoreHandle core, EvimViewId view,
                                    const uint8_t *text, uint64_t text_length,
                                    EvimCoreOutcomeV1 *out_outcome);

/*
 * Host-context turns consume immutable per-turn clipboard snapshots and
 * writable capabilities. On success, out_effect_batch is zero or an owned
 * handle which must be released. The effect batch survives core destruction.
 */
EvimStatus evim_core_view_send_key_with_host_context(
    EvimCoreHandle core, EvimViewId view, const EvimKeyInputV1 *input,
    const EvimCommandTurnContextV1 *context,
    EvimCoreOutcomeV1 *out_outcome,
    EvimEffectBatchHandle *out_effect_batch);
EvimStatus evim_core_view_send_text_with_host_context(
    EvimCoreHandle core, EvimViewId view, const uint8_t *text,
    uint64_t text_length, const EvimCommandTurnContextV1 *context,
    EvimCoreOutcomeV1 *out_outcome,
    EvimEffectBatchHandle *out_effect_batch);

EvimStatus evim_effect_batch_info(EvimEffectBatchHandle batch,
                                  EvimEffectBatchInfoV1 *out_info);
/*
 * Copy is all-or-none across every typed array and the UTF-8 arena. A short
 * capacity returns BUFFER_TOO_SMALL plus complete required sizes and writes no
 * array/arena element. These are raw Ex requests: the batch performs no I/O,
 * does not own a prepared artifact-write lifecycle, and does not establish a
 * save point. Route native work and acknowledge exact successful saves through
 * evim_core_mark_saved separately.
 */
EvimStatus evim_effect_batch_copy(
    EvimEffectBatchHandle batch,
    EvimClipboardWriteV1 *clipboard_writes,
    uint64_t clipboard_write_capacity,
    EvimExFrontendRequestV1 *ex_requests,
    uint64_t ex_request_capacity,
    EvimExOptionDisplayV1 *ex_options,
    uint64_t ex_option_capacity,
    EvimExMarkV1 *ex_marks,
    uint64_t ex_mark_capacity,
    EvimExRegisterV1 *ex_registers,
    uint64_t ex_register_capacity,
    EvimExJumpV1 *ex_jumps,
    uint64_t ex_jump_capacity,
    EvimExTextLineV1 *ex_text_lines,
    uint64_t ex_text_line_capacity,
    uint32_t *file_formats,
    uint64_t file_format_capacity,
    uint64_t *hard_breaks,
    uint64_t hard_break_capacity,
    uint8_t *string_bytes,
    uint64_t string_capacity,
    EvimEffectBatchInfoV1 *out_info);
EvimStatus evim_effect_batch_release(EvimEffectBatchHandle batch);

EvimStatus evim_core_view_place_cursor(
    EvimCoreHandle core, EvimViewId view,
    const EvimPlaceCursorV1 *request, EvimCoreOutcomeV1 *out_outcome);

/* Native Edit-menu history navigation; behavior is independent of Vim mode. */
EvimStatus evim_core_view_undo(EvimCoreHandle core, EvimViewId view,
                               EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_redo(EvimCoreHandle core, EvimViewId view,
                               EvimCoreOutcomeV1 *out_outcome);

/*
 * Composition requests are exact-revision operations. Begin replacement and
 * update selection ranges use UTF-8 byte offsets and must also be extended-
 * grapheme boundaries. Update and cancel never edit source. Commit applies its
 * final UTF-8 payload as one source transaction and one undo unit. On a model
 * policy failure, source stays exact and the final payload remains marked so
 * the frontend can cancel or retry under another policy.
 */
EvimStatus evim_core_view_composition_begin(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionBeginV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_update(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionUpdateV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_overlay_info(
    EvimCoreHandle core, EvimViewId view,
    EvimCompositionOverlayInfoV1 *out_info);
EvimStatus evim_core_view_copy_composition_utf8_range(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionOverlayUtf8RangeV1 *request, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
EvimStatus evim_core_view_composition_commit(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionCommitV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_cancel(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionCancelV1 *request, EvimCoreOutcomeV1 *out_outcome);

/*
 * Horizontal-only origin changes do not shape, reflow, or replace immutable
 * layout. HAS_TOP validates the supplied identity and atomically installs one
 * bounded regional viewport; a stale identity changes no view state.
 */
EvimStatus evim_core_view_set_viewport_origin(
    EvimCoreHandle core, EvimViewId view,
    const EvimViewportOriginV1 *request, EvimCoreOutcomeV1 *out_outcome);

EvimStatus evim_core_view_resize(EvimCoreHandle core, EvimViewId view,
                                 float width, float height,
                                 EvimCoreOutcomeV1 *out_outcome);
/*
 * Change only this view's magnification. The scale must be finite and greater
 * than zero. Shaping/wrapping/layout are refreshed synchronously; source and
 * semantic projections are unchanged.
 */
EvimStatus evim_core_view_set_scale(EvimCoreHandle core, EvimViewId view,
                                    float scale,
                                    EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_set_wrap(EvimCoreHandle core, EvimViewId view,
                                   uint32_t wrap,
                                   EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_set_linebreak(EvimCoreHandle core, EvimViewId view,
                                        uint32_t linebreak,
                                        EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_set_file_format(
    EvimCoreHandle core, EvimViewId view,
    const EvimSetFileFormatV1 *request, EvimCoreOutcomeV1 *out_outcome);

/*
 * Begin one exact frontend-owned style gesture. Only one may be active in a
 * core. An empty group creates no history entry. Any ordinary coordinator
 * event or removal of the owner closes the successful prefix and consumes the
 * capability before continuing.
 */
EvimStatus evim_core_view_begin_style_edit_group(
    EvimCoreHandle core, EvimViewId view,
    const EvimStyleSheetIdentityV1 *expected,
    EvimStyleEditGroupV1 *out_group);
/* One exact generated-definition field edit and standalone undo unit. */
EvimStatus evim_core_view_edit_style(
    EvimCoreHandle core, EvimViewId view,
    const EvimStyleEditV1 *request, EvimCoreOutcomeV1 *out_outcome);
/*
 * Commit one immediately visible exact edit into group. request must carry the
 * current style-sheet identity on every call. A failed/no-op edit leaves the
 * group usable and does not poison previously successful edits.
 */
EvimStatus evim_core_view_edit_style_in_group(
    EvimCoreHandle core, EvimViewId view,
    const EvimStyleEditGroupV1 *group,
    const EvimStyleEditV1 *request, EvimCoreOutcomeV1 *out_outcome);
/*
 * Consume group and close its successful edits as one undo/redo unit. This
 * never rolls back. Reuse after end returns INVALID_STYLE_EDIT_GROUP.
 */
EvimStatus evim_core_view_end_style_edit_group(
    EvimCoreHandle core, EvimViewId view,
    const EvimStyleEditGroupV1 *group);

EvimStatus evim_core_copy_source_bytes(EvimCoreHandle core,
                                       uint64_t expected_revision,
                                       uint8_t *output,
                                       uint64_t output_capacity,
                                       uint64_t *out_required);
EvimStatus evim_core_copy_formatted_utf8(EvimCoreHandle core,
                                         uint64_t expected_revision,
                                         uint8_t *output,
                                         uint64_t output_capacity,
                                         uint64_t *out_required);

#ifdef __cplusplus
}
#endif

#endif /* EVIM_CORE_H */
