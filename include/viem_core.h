#ifndef VIEM_CORE_H
#define VIEM_CORE_H

#include <stdint.h>
#include "viem_startup.h"

#ifdef __cplusplus
extern "C" {
#endif

#define VIEM_CORE_ABI_VERSION 8u
#define VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V4 4u
#define VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION \
  VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V4

typedef uint64_t ViemCoreHandle;
/* Owned immutable command-turn effects; zero means no effects. */
typedef uint64_t ViemEffectBatchHandle;
/* Owned immutable standalone HTML export; release after copying its bytes. */
typedef uint64_t ViemHtmlExportHandle;
typedef uint64_t ViemViewId;
typedef uint32_t ViemStatus;

#define VIEM_STATUS_OK 0u
#define VIEM_STATUS_INVALID_ARGUMENT 1u
#define VIEM_STATUS_NULL_POINTER 2u
#define VIEM_STATUS_INVALID_HANDLE 3u
#define VIEM_STATUS_STALE_REVISION 4u
#define VIEM_STATUS_INVALID_UTF8 5u
#define VIEM_STATUS_INVALID_ENCODING 6u
#define VIEM_STATUS_INVALID_FORMAT 7u
#define VIEM_STATUS_INVALID_FILE_FORMAT 8u
#define VIEM_STATUS_BUFFER_TOO_SMALL 9u
#define VIEM_STATUS_INVALID_RANGE 10u
#define VIEM_STATUS_NOT_GRAPHEME_BOUNDARY 11u
#define VIEM_STATUS_UNREPRESENTABLE_CHARACTER 12u
#define VIEM_STATUS_AMBIGUOUS_PROJECTION 13u
#define VIEM_STATUS_UNSUPPORTED_OPERATION 14u
#define VIEM_STATUS_POLICY_REQUIRED 15u
#define VIEM_STATUS_RESOURCE_EXHAUSTED 16u
#define VIEM_STATUS_VERIFICATION_FAILED 17u
#define VIEM_STATUS_LENGTH_OVERFLOW 18u
#define VIEM_STATUS_CORE_BUSY 20u
#define VIEM_STATUS_INVALID_VIEW 21u
#define VIEM_STATUS_INVALID_PROVIDER 22u
#define VIEM_STATUS_PROVIDER_FAILURE 23u
#define VIEM_STATUS_INVALID_KEY 24u
#define VIEM_STATUS_CORE_FAILURE 25u
#define VIEM_STATUS_UNSTABLE_SHAPING_CONTEXT 26u
#define VIEM_STATUS_LAYOUT_UNAVAILABLE 28u
#define VIEM_STATUS_OUTSIDE_LAYOUT_COVERAGE 29u
#define VIEM_STATUS_UNKNOWN_STYLE 30u
#define VIEM_STATUS_STYLE_READ_ONLY 31u
#define VIEM_STATUS_INVALID_STYLE_VALUE 32u
#define VIEM_STATUS_STYLE_INHERITANCE_CYCLE 33u
#define VIEM_STATUS_INCOMPATIBLE_STYLE_ROLE 34u
#define VIEM_STATUS_INVALID_STYLE_RELATIONSHIP 35u
#define VIEM_STATUS_INVALID_UTF8_BOUNDARY 36u
#define VIEM_STATUS_INVALID_UTF16_BOUNDARY 37u
#define VIEM_STATUS_STYLE_EDIT_GROUP_ACTIVE 38u
#define VIEM_STATUS_INVALID_STYLE_EDIT_GROUP 39u
#define VIEM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER 40u
#define VIEM_STATUS_INTERNAL_ERROR 254u
#define VIEM_STATUS_PANIC 255u

/* Frontend-owned window geometry; root pane ID is 1. IDs/handles never reuse.
 * Orientations: 0 stacked, 1 side by side. At most 256 panes (511 frames).
 * Failed mutations leave geometry unchanged. All calls are serial per handle.
 * Copy outputs must be aligned and disjoint; policy-required means no room. */
typedef struct ViemPaneChrome { uint64_t id; double status_height; } ViemPaneChrome;
typedef struct ViemPaneFrame {
  uint64_t id; uint32_t kind; uint32_t flags;
  double x; double y; double width; double height;
} ViemPaneFrame;
typedef struct ViemPaneSnapshot { uint64_t count; double minimum_width; double minimum_height; } ViemPaneSnapshot;
ViemStatus viem_pane_layout_create(uint64_t *out_handle);
ViemStatus viem_pane_layout_destroy(uint64_t handle);
ViemStatus viem_pane_layout_update(uint64_t handle, double width, double height, const ViemPaneChrome *chrome, uint64_t count);
ViemStatus viem_pane_layout_copy(uint64_t handle, ViemPaneFrame *frames, uint64_t capacity, ViemPaneSnapshot *out_snapshot);
ViemStatus viem_pane_layout_can_split(uint64_t handle, uint64_t pane, uint32_t orientation, double status_height);
ViemStatus viem_pane_layout_split(uint64_t handle, uint64_t pane, uint32_t orientation, double status_height, uint64_t *out_pane);
ViemStatus viem_pane_layout_remove(uint64_t handle, uint64_t pane);
ViemStatus viem_pane_layout_drag(uint64_t handle, uint64_t id, uint32_t kind, double delta);
/* Operations: focus=1(direction=count, steps=value, caret=x/y), rotate=2
 * (steps=count, flags:0 forward/1 backward), exchange=3(one-based count, 0=next), move=4(direction=count),
 * resize=5(axis=count, DIPs=value, flags:1 relative/2 maximize), equalize=6
 * (count:0 both/1 heights/2 widths). Directions:0 down/1 up/2 left/3 right. */
ViemStatus viem_pane_layout_action(uint64_t handle, uint64_t pane, uint32_t operation, uint64_t count, double value, uint32_t flags, double x, double y, uint64_t *out_focus);

/*
 * Core-owned initial detection: supported BOM first, otherwise wholly valid
 * UTF-8, otherwise ISO-8859-1. Nonzero values force the named encoding.
 */
#define VIEM_ENCODING_DETECT 0u
#define VIEM_ENCODING_UTF8 1u
#define VIEM_ENCODING_LATIN1 2u
#define VIEM_ENCODING_UTF16_LE 3u
#define VIEM_ENCODING_UTF16_BE 4u

#define VIEM_FORMAT_PLAIN_TEXT 1u
#define VIEM_FORMAT_MARKDOWN 2u
#define VIEM_FORMAT_MARKDOWN_SOURCE 5u
#define VIEM_FORMAT_CODE 7u

#define VIEM_FILE_FORMAT_DETECT 0u
#define VIEM_FILE_FORMAT_UNIX 1u
#define VIEM_FILE_FORMAT_DOS 2u
#define VIEM_FILE_FORMAT_MAC 3u

#define VIEM_FILE_FORMAT_ORIGIN_DETECTED 1u
#define VIEM_FILE_FORMAT_ORIGIN_FORCED 2u
#define VIEM_FILE_FORMAT_ORIGIN_DEFAULTED 3u

#define VIEM_HISTORY_ACTION_CATEGORY_NONE 0u
#define VIEM_HISTORY_ACTION_CATEGORY_TEXT 1u
#define VIEM_HISTORY_ACTION_CATEGORY_STYLE 2u
#define VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT 3u
#define VIEM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER 4u
#define VIEM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA 6u
#define VIEM_HISTORY_ACTION_CATEGORY_MIXED 7u

#define VIEM_DOCUMENT_STATE_HAS_BOM (1u << 0)
#define VIEM_DOCUMENT_STATE_CAN_UNDO (1u << 1)
#define VIEM_DOCUMENT_STATE_CAN_REDO (1u << 2)
#define VIEM_DOCUMENT_STATE_IS_DIRTY (1u << 3)
#define VIEM_DOCUMENT_STATE_READ_ONLY (1u << 4)
#define VIEM_DOCUMENT_STATE_RECOVERED (1u << 5)

typedef struct ViemDocumentOptions {
  uint32_t struct_size;
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
} ViemDocumentOptions;

#define VIEM_DOCUMENT_OPTIONS_SIZE 16u

/* Model, pipeline, and history metadata captured in one serial core query. */
typedef struct ViemDocumentStateV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
  uint64_t source_byte_count;
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
  uint32_t file_format_origin;
  uint32_t undo_action_category;
  uint32_t redo_action_category;
  uint32_t reserved[2];
} ViemDocumentStateV1;

#define VIEM_DOCUMENT_STATE_V1_SIZE \
  ((uint32_t)sizeof(ViemDocumentStateV1))

/* Exact identity of one immutable formatted projection. */
typedef struct ViemFormattedSnapshotIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
} ViemFormattedSnapshotIdentityV1;

#define VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemFormattedSnapshotIdentityV1))

/* Constant-time aggregate metadata; querying it never flattens document text. */
typedef struct ViemFormattedSnapshotInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t utf16_length;
  uint64_t hard_line_count;
} ViemFormattedSnapshotInfoV1;

#define VIEM_FORMATTED_SNAPSHOT_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemFormattedSnapshotInfoV1))

/* Scalar-aligned, half-open UTF-8 range in one exact formatted snapshot. */
typedef struct ViemFormattedUtf8RangeV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_start;
  uint64_t utf8_end;
} ViemFormattedUtf8RangeV1;

#define VIEM_FORMATTED_UTF8_RANGE_V1_SIZE \
  ((uint32_t)sizeof(ViemFormattedUtf8RangeV1))

/*
 * Logical status metadata for one grapheme-aligned formatted point. Line and
 * column values are zero based. hard_line_start..hard_line_end excludes the
 * following semantic hard break.
 */
typedef struct ViemFormattedPointInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemFormattedSnapshotIdentityV1 identity;
  uint64_t utf8_offset;
  uint64_t utf16_offset;
  uint64_t hard_line_index;
  /* For READ, hard_line_start is the one-based line after which to insert;
     zero inserts before the first line. It is not a selected range. */
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
} ViemFormattedPointInfoV1;

#define VIEM_FORMATTED_POINT_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemFormattedPointInfoV1))

#define VIEM_PROVIDER_THREADING_ANY_WORKER 1u
#define VIEM_PROVIDER_THREADING_DEDICATED_SERIAL 2u
#define VIEM_PROVIDER_THREADING_FRONTEND_MAIN 3u

#define VIEM_RENDER_THREADING_ANY 1u
#define VIEM_RENDER_THREADING_DEDICATED_SERIAL 2u
#define VIEM_RENDER_THREADING_FRONTEND_MAIN 3u

#define VIEM_LAYOUT_EXECUTION_WORKER_POOL 1u
#define VIEM_LAYOUT_EXECUTION_DEDICATED_SERIAL 2u
#define VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN 3u

#define VIEM_TEXT_DIRECTION_AUTO 0u
#define VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT 1u
#define VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT 2u

#define VIEM_FONT_SLANT_UPRIGHT 0u
#define VIEM_FONT_SLANT_ITALIC 1u
#define VIEM_FONT_SLANT_OBLIQUE 2u

#define VIEM_SHAPE_PURPOSE_METRICS_ONLY 1u
#define VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA 2u

#define VIEM_BOUNDARY_AFFINITY_UPSTREAM 1u
#define VIEM_BOUNDARY_AFFINITY_DOWNSTREAM 2u

#define VIEM_KEY_CHARACTER 1u
#define VIEM_KEY_ESCAPE 2u
#define VIEM_KEY_ENTER 3u
#define VIEM_KEY_TAB 4u
#define VIEM_KEY_BACKSPACE 5u
#define VIEM_KEY_DELETE 6u
#define VIEM_KEY_LEFT 7u
#define VIEM_KEY_RIGHT 8u
#define VIEM_KEY_UP 9u
#define VIEM_KEY_DOWN 10u
#define VIEM_KEY_HOME 11u
#define VIEM_KEY_END 12u
#define VIEM_KEY_PAGE_UP 13u
#define VIEM_KEY_PAGE_DOWN 14u
#define VIEM_KEY_CONTROL_CHARACTER 15u
#define VIEM_KEY_BACK_TAB 16u
#define VIEM_KEY_DOCUMENT_START 17u
#define VIEM_KEY_DOCUMENT_END 18u
#define VIEM_KEY_SHIFT_ENTER 19u
#define VIEM_KEY_WORD_LEFT 20u
#define VIEM_KEY_WORD_RIGHT 21u
#define VIEM_KEY_FUNCTION 22u
#define VIEM_KEY_COPY_SELECTION 23u
#define VIEM_KEY_PARAGRAPH_START 24u
#define VIEM_KEY_PARAGRAPH_END 25u
#define VIEM_KEY_NEXT_PARAGRAPH 26u
#define VIEM_KEY_MODIFIER_SHIFT 1u
#define VIEM_KEY_MODIFIER_CONTROL 2u
#define VIEM_KEY_MODIFIER_ALT 4u
#define VIEM_KEY_MODIFIER_COMMAND 8u

#define VIEM_COMMAND_STATUS_NONE 0u
#define VIEM_COMMAND_STATUS_COMPLETE 1u
#define VIEM_COMMAND_STATUS_PENDING 2u
#define VIEM_COMMAND_STATUS_CANCELLED 3u
#define VIEM_COMMAND_STATUS_NEEDS_MORE_LAYOUT 4u
#define VIEM_COMMAND_STATUS_SEARCH_NOT_FOUND 5u
#define VIEM_COMMAND_STATUS_UNSUPPORTED 6u
#define VIEM_COMMAND_STATUS_ERROR 7u
#define VIEM_COMMAND_STATUS_READ_ONLY 8u

#define VIEM_MODE_NORMAL 1u
#define VIEM_MODE_INSERT 2u
#define VIEM_MODE_REPLACE 3u
#define VIEM_MODE_VISUAL_CHARACTER 4u
#define VIEM_MODE_VISUAL_LINE 5u
#define VIEM_MODE_VISUAL_BLOCK 6u
#define VIEM_MODE_COMMAND_LINE 7u
#define VIEM_MODE_SELECTION_CHARACTER 11u
#define VIEM_MODE_SELECTION_LINE 12u
#define VIEM_MODE_SELECTION_BLOCK 13u
#define VIEM_MODE_SELECT_CHARACTER 8u
#define VIEM_MODE_SELECT_LINE 9u
#define VIEM_MODE_SELECT_BLOCK 10u

#define VIEM_CLIPBOARD_TARGET_CLIPBOARD 1u
#define VIEM_CLIPBOARD_TARGET_PRIMARY 2u

#define VIEM_CLIPBOARD_TURN_HAS_READ (1u << 0)
#define VIEM_CLIPBOARD_TURN_WRITABLE (1u << 1)

#define VIEM_EFFECT_BATCH_HAS_EX_OUTCOME (1u << 0)
#define VIEM_EFFECT_BATCH_EX_DOCUMENT_CHANGED (1u << 1)
#define VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION (1u << 2)
#define VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY (1u << 3)

#define VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER (1u << 0)
#define VIEM_REGISTER_KIND_NONE 0u
#define VIEM_REGISTER_KIND_CHARACTER 1u
#define VIEM_REGISTER_KIND_LINE 2u
#define VIEM_REGISTER_KIND_BLOCK 3u

#define VIEM_EX_FRONTEND_EDIT 1u
#define VIEM_EX_FRONTEND_NEW 2u
#define VIEM_EX_FRONTEND_WRITE 3u
#define VIEM_EX_FRONTEND_SAVE_AS 4u
#define VIEM_EX_FRONTEND_QUIT 5u
#define VIEM_EX_FRONTEND_QUIT_ALL 6u
#define VIEM_EX_FRONTEND_WRITE_QUIT 7u
#define VIEM_EX_FRONTEND_XIT 8u
#define VIEM_EX_FRONTEND_WRITE_ALL 9u
#define VIEM_EX_FRONTEND_MARKS 10u
#define VIEM_EX_FRONTEND_REGISTERS 11u
#define VIEM_EX_FRONTEND_JUMPS 12u
#define VIEM_EX_FRONTEND_OPTIONS 13u
#define VIEM_EX_FRONTEND_PRINT_LINES 14u
#define VIEM_EX_FRONTEND_NORMAL 15u
#define VIEM_EX_FRONTEND_SPLIT 16u
#define VIEM_EX_FRONTEND_MESSAGE 17u
#define VIEM_EX_FRONTEND_EDIT_NEW_WINDOW 18u
#define VIEM_EX_FRONTEND_PWD 19u
#define VIEM_EX_FRONTEND_CD 20u
#define VIEM_EX_FRONTEND_CHECKTIME 21u
/* A CTRL-W window effect. window_command names it; window_count carries its
   count or one-based pane index when VIEM_EX_FRONTEND_HAS_COUNT is set. */
#define VIEM_EX_FRONTEND_WINDOW 22u
#define VIEM_EX_FRONTEND_NEW_PANE 23u
#define VIEM_EX_FRONTEND_ARGUMENT 24u
#define VIEM_EX_FRONTEND_READ 25u
#define VIEM_EX_FRONTEND_SOURCE 26u
#define VIEM_EX_FRONTEND_FILE 27u
#define VIEM_EX_FRONTEND_ONLY 28u
/* Quit without writing; window_count carries the process exit status. */
#define VIEM_EX_FRONTEND_CQUIT 29u
#define VIEM_ARGUMENT_NEXT 1u
#define VIEM_ARGUMENT_PREVIOUS 2u
#define VIEM_ARGUMENT_FIRST 3u
#define VIEM_ARGUMENT_LAST 4u
#define VIEM_ARGUMENT_INDEX 5u
#define VIEM_ARGUMENT_CURRENT 6u
#define VIEM_ARGUMENT_RESOLVE_OK 0u
#define VIEM_ARGUMENT_RESOLVE_EMPTY 1u
#define VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST 2u
#define VIEM_ARGUMENT_RESOLVE_AFTER_LAST 3u
#define VIEM_ARGUMENT_RESOLVE_INVALID_INDEX 4u

typedef struct ViemArgumentResolution {
  uint32_t status;
  uint32_t reserved;
  uint64_t index;
} ViemArgumentResolution;

/* Current and remembered indices are zero based; UINT64_MAX means absent.
 * Count is a positive step count or one-based index according to command. */
ViemArgumentResolution viem_argument_list_resolve(
    uint64_t length, uint64_t current_index, uint64_t remembered_index,
    uint32_t command, uint64_t count);

/* Owned, serial external-file review state. Handles are nonzero and never
 * reused. Destroy consumes ownership only on success; CORE_BUSY retains it. */
typedef uint64_t ViemExternalFileReviewHandle;
#define VIEM_EXTERNAL_FILE_REVIEW_PRESENT (1u << 0)
#define VIEM_EXTERNAL_FILE_REVIEW_CAN_RELOAD (1u << 1)
#define VIEM_EXTERNAL_FILE_REVIEW_DISCARDS_UNSAVED_CHANGES (1u << 2)
#define VIEM_MAX_EXTERNAL_FILE_OBSERVATION_BYTES 4096u
ViemStatus viem_external_file_review_create(ViemExternalFileReviewHandle *out_handle);
ViemStatus viem_external_file_review_destroy(ViemExternalFileReviewHandle handle);
ViemStatus viem_external_file_review_reset(ViemExternalFileReviewHandle handle);
/* Call after observing the saved baseline again. Clears acknowledgement only;
 * an active review remains pending and its decision still needs validation. */
ViemStatus viem_external_file_review_clear_acknowledged(ViemExternalFileReviewHandle handle);
/* Tokens are opaque, nonempty, length-delimited bytes, at most the maximum
 * above. All Boolean inputs are 0/1. Input/output regions must be disjoint.
 * Successful begin returns zero flags for an already acknowledged observation
 * or while another review is active. Otherwise PRESENT and action flags apply. */
ViemStatus viem_external_file_review_begin(
    ViemExternalFileReviewHandle handle, const uint8_t *token, uint64_t token_length,
    uint32_t can_reload, uint32_t is_dirty, uint32_t *out_flags);
/* Only a matching active review is completed; mismatched/invalidated reviews
 * are no-ops. Acknowledge 0 releases for retry; 1 suppresses this observation. */
ViemStatus viem_external_file_review_finish(
    ViemExternalFileReviewHandle handle, const uint8_t *token, uint64_t token_length,
    uint32_t acknowledge);

#define VIEM_WINDOW_FOCUS_DOWN 1u
#define VIEM_WINDOW_FOCUS_UP 2u
#define VIEM_WINDOW_FOCUS_NEXT 3u
#define VIEM_WINDOW_FOCUS_PREVIOUS 4u
#define VIEM_WINDOW_FOCUS_TOP 5u
#define VIEM_WINDOW_FOCUS_BOTTOM 6u
#define VIEM_WINDOW_FOCUS_LAST_ACCESSED 7u
#define VIEM_WINDOW_ROTATE_DOWN 8u
#define VIEM_WINDOW_ROTATE_UP 9u
#define VIEM_WINDOW_EXCHANGE 10u
#define VIEM_WINDOW_MOVE_TO_TOP 11u
#define VIEM_WINDOW_MOVE_TO_BOTTOM 12u
#define VIEM_WINDOW_CLOSE_OTHERS 13u
#define VIEM_WINDOW_GROW 14u
#define VIEM_WINDOW_SHRINK 15u
#define VIEM_WINDOW_SET_HEIGHT 16u
#define VIEM_WINDOW_EQUALIZE_HEIGHTS 17u
#define VIEM_WINDOW_FOCUS_LEFT 18u
#define VIEM_WINDOW_FOCUS_RIGHT 19u
#define VIEM_WINDOW_MOVE_TO_LEFT 20u
#define VIEM_WINDOW_MOVE_TO_RIGHT 21u
#define VIEM_WINDOW_GROW_WIDTH 22u
#define VIEM_WINDOW_SHRINK_WIDTH 23u
#define VIEM_WINDOW_SET_WIDTH 24u
#define VIEM_WINDOW_EQUALIZE_HEIGHT_ONLY 25u
#define VIEM_WINDOW_EQUALIZE_WIDTH_ONLY 26u
#define VIEM_WINDOW_RESIZE_INDEXED 27u
/* Indexed resize: argument_count is the target pane (0=current), argument_command
 * is 0 absolute/1 grow/2 shrink; window_count is the size when HAS_COUNT. */
#define VIEM_EX_FRONTEND_VERTICAL (1u << 9)


#define VIEM_EX_FRONTEND_FORCE (1u << 0)
#define VIEM_EX_FRONTEND_HAS_PATH (1u << 1)
#define VIEM_EX_FRONTEND_HAS_RANGE (1u << 2)
#define VIEM_EX_FRONTEND_NUMBER (1u << 3)
#define VIEM_EX_FRONTEND_LIST (1u << 4)
#define VIEM_EX_FRONTEND_LITERAL (1u << 5)
/* window_count carries an explicit count or pane index. */
#define VIEM_EX_FRONTEND_HAS_COUNT (1u << 6)
#define VIEM_EX_FRONTEND_WRITE_FIRST (1u << 7)
#define VIEM_EX_FRONTEND_HAS_LINE (1u << 8)

#define VIEM_EX_OPTION_WRAP 1u
#define VIEM_EX_OPTION_LINEBREAK 2u
#define VIEM_EX_OPTION_FILE_FORMAT 3u
#define VIEM_EX_OPTION_FILE_FORMATS 4u
#define VIEM_EX_OPTION_IGNORECASE 5u
#define VIEM_EX_OPTION_SMARTCASE 6u
#define VIEM_EX_OPTION_WRAPSCAN 7u
#define VIEM_EX_OPTION_TEXTWIDTH 8u
#define VIEM_EX_OPTION_AUTOINDENT 9u
#define VIEM_EX_OPTION_TABSTOP 10u
#define VIEM_EX_OPTION_SHIFTWIDTH 11u
#define VIEM_EX_OPTION_SOFTTABSTOP 12u
#define VIEM_EX_OPTION_EXPANDTAB 13u
#define VIEM_EX_OPTION_SMARTTAB 14u
#define VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_ENTER 15u
#define VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_OPEN_LINE 16u


#define VIEM_EX_OPTION_VALUE_BOOLEAN 1u
#define VIEM_EX_OPTION_VALUE_FILE_FORMAT 2u
#define VIEM_EX_OPTION_VALUE_FILE_FORMATS 3u
/* scalar_value carries the number. */
#define VIEM_EX_OPTION_VALUE_NUMBER 4u
#define VIEM_EX_OPTION_VALUE_STRING 5u
#define VIEM_EX_OPTION_LIST 17u
#define VIEM_EX_OPTION_LISTCHARS 18u
#define VIEM_EX_OPTION_KEYMODEL 21u
#define VIEM_EX_OPTION_SELECTMODE 22u
#define VIEM_EX_OPTION_AUTOSELECT 23u
#define VIEM_EX_OPTION_HLSEARCH 19u
#define VIEM_EX_OPTION_INCSEARCH 20u

#define VIEM_EX_JUMP_CURRENT (1u << 0)

#define VIEM_OUTCOME_HAS_COMMAND (1u << 0)
#define VIEM_OUTCOME_CURSOR_MOVED (1u << 1)
#define VIEM_OUTCOME_DOCUMENT_CHANGED (1u << 2)
#define VIEM_OUTCOME_MODE_CHANGED (1u << 3)
#define VIEM_OUTCOME_LAYOUT_CHANGED (1u << 4)
#define VIEM_OUTCOME_HAS_POSITION_MAP (1u << 5)
#define VIEM_OUTCOME_HAS_LAYOUT (1u << 6)
#define VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS (1u << 7)
#define VIEM_OUTCOME_HAS_COMPOSITION_CHANGES (1u << 8)

typedef struct ViemUtf8Slice {
  const uint8_t *data;
  uint64_t length;
} ViemUtf8Slice;

/* One snapshot/capability per clipboard target, with optional private JSON. */
typedef struct ViemClipboardTurnEntryV2 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t target;
  uint32_t reserved;
  uint64_t generation;
  ViemUtf8Slice plain_text;
  ViemUtf8Slice fragment_json;
} ViemClipboardTurnEntryV2;
#define VIEM_CLIPBOARD_TURN_ENTRY_V2_SIZE ((uint32_t)sizeof(ViemClipboardTurnEntryV2))
typedef struct ViemCommandTurnContextV2 {
  uint32_t struct_size;
  uint32_t reserved;
  const ViemClipboardTurnEntryV2 *clipboards;
  uint64_t clipboard_count;
} ViemCommandTurnContextV2;
#define VIEM_COMMAND_TURN_CONTEXT_V2_SIZE ((uint32_t)sizeof(ViemCommandTurnContextV2))

/* Offset and length in the UTF-8 string arena copied with an effect batch. */
typedef struct ViemEffectBytesRefV1 {
  uint64_t offset;
  uint64_t length;
} ViemEffectBytesRefV1;

typedef struct ViemClipboardWriteV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t target;
  uint32_t register_kind;
  uint64_t document_id;
  uint64_t document_revision;
  ViemEffectBytesRefV1 plain_text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} ViemClipboardWriteV1;

#define VIEM_CLIPBOARD_WRITE_V1_SIZE \
  ((uint32_t)sizeof(ViemClipboardWriteV1))

/* Subordinate display value for one VIEM_EX_FRONTEND_OPTIONS request. */
typedef struct ViemExOptionDisplayV1 {
  uint32_t struct_size;
  uint32_t name;
  uint32_t value_kind;
  uint32_t scalar_value;
  uint64_t first_file_format;
  uint64_t file_format_count;
  ViemEffectBytesRefV1 text;
} ViemExOptionDisplayV1;

#define VIEM_EX_OPTION_DISPLAY_V1_SIZE \
  ((uint32_t)sizeof(ViemExOptionDisplayV1))

/* Exact resolved mark plus its captured hard-line text. */
typedef struct ViemExMarkV1 {
  uint32_t struct_size;
  uint32_t name;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  ViemEffectBytesRefV1 line_text;
} ViemExMarkV1;

#define VIEM_EX_MARK_V1_SIZE ((uint32_t)sizeof(ViemExMarkV1))

/* Exact resolved register contents and optional semantic hard-break offsets. */
typedef struct ViemExRegisterV1 {
  uint32_t struct_size;
  uint32_t name;
  uint32_t register_kind;
  uint32_t reserved;
  ViemEffectBytesRefV1 text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} ViemExRegisterV1;

#define VIEM_EX_REGISTER_V1_SIZE ((uint32_t)sizeof(ViemExRegisterV1))

/* Exact jump in oldest-to-newest order plus its captured hard-line text. */
typedef struct ViemExJumpV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t list_index;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  ViemEffectBytesRefV1 line_text;
} ViemExJumpV1;

#define VIEM_EX_JUMP_V1_SIZE ((uint32_t)sizeof(ViemExJumpV1))

/* Exact post-turn formatted hard-line content for PRINT_LINES. */
typedef struct ViemExTextLineV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t hard_line_index;
  uint64_t utf8_start;
  uint64_t utf8_end;
  ViemEffectBytesRefV1 text;
} ViemExTextLineV1;

#define VIEM_EX_TEXT_LINE_V1_SIZE ((uint32_t)sizeof(ViemExTextLineV1))

/*
 * One raw Ex request in execution order. text references the copied UTF-8
 * arena and is an optional path, :normal command string, or concatenated
 * Unicode mark/register-name sequence according to kind. Ranges are inclusive
 * zero-based hard-line indices. Option indices address the option array.
 * first_payload/payload_count address the mark, register, jump, or text-line
 * array selected by the info-request kind.
 */
typedef struct ViemExFrontendRequestV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t flags;
  /* One VIEM_WINDOW_* value when kind is VIEM_EX_FRONTEND_WINDOW. */
  uint32_t window_command;
  uint64_t document_id;
  uint64_t document_revision;
  ViemEffectBytesRefV1 text;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t first_option;
  uint64_t option_count;
  uint64_t first_payload;
  uint64_t payload_count;
  /* With VIEM_EX_FRONTEND_HAS_COUNT: window count/index, or initial height
     in default paragraph lines for SPLIT and NEW_PANE. */
  uint64_t window_count;
  /* VIEM_ARGUMENT_* command, count and optional initial line for ARGUMENT. */
  uint32_t argument_command;
  uint32_t reserved;
  uint64_t argument_count;
  /* One-based line; zero means last line. Valid only with HAS_LINE. */
  uint64_t argument_line;
} ViemExFrontendRequestV1;

#define VIEM_EX_FRONTEND_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(ViemExFrontendRequestV1))

/* Exact immutable batch identity, flags, and two-pass copy sizes. */
typedef struct ViemEffectBatchInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemEffectBatchHandle batch_handle;
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
} ViemEffectBatchInfoV1;

#define VIEM_EFFECT_BATCH_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemEffectBatchInfoV1))

typedef struct ViemTextMetricsV1 {
  float ascent;
  float descent;
  float leading;
} ViemTextMetricsV1;

typedef struct ViemShapedBoundsV1 {
  float x;
  float y;
  float width;
  float height;
} ViemShapedBoundsV1;

typedef struct ViemOpenTypeFeatureV1 {
  uint8_t tag[4];
  uint32_t value;
} ViemOpenTypeFeatureV1;

typedef struct ViemResolvedTextStyleV1 {
  uint32_t struct_size;
  uint32_t slant;
  uint32_t direction;
  uint32_t has_language;
  uint32_t has_script;
  /* bit0: weight includes relative bold (+300); retain selected face as base.
     Other bits are reserved and zero. Older providers may ignore this hint. */
  uint32_t reserved;
  float size;
  float weight;
  float letter_spacing;
  const ViemUtf8Slice *font_families;
  uint64_t font_family_count;
  ViemUtf8Slice language;
  ViemUtf8Slice script;
  const ViemOpenTypeFeatureV1 *features;
  uint64_t feature_count;
  ViemUtf8Slice font_axes; /* JSON object: four-character axis tags to finite coordinates. */
  ViemUtf8Slice font_face; /* OpenType subfamily of the primary family; empty for automatic selection. */
} ViemResolvedTextStyleV1;

#define VIEM_RESOLVED_TEXT_STYLE_RELATIVE_BOLD (1u << 0)

#define VIEM_RESOLVED_TEXT_STYLE_V1_SIZE \
  ((uint32_t)sizeof(ViemResolvedTextStyleV1))

typedef struct ViemShapeStyleRunV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  ViemResolvedTextStyleV1 style;
} ViemShapeStyleRunV1;

#define VIEM_SHAPE_STYLE_RUN_V1_SIZE ((uint32_t)sizeof(ViemShapeStyleRunV1))

/*
 * Provider-owned opaque render resource. identifier is an integer token, not
 * a native pointer for core to dereference. Response arenas pin resources until
 * the next shape call. Core retains an independent fragment lease shared by
 * caches and snapshots. Tokens remain usable while leased and their generation
 * is current. Borrowed frontend exports remain tied to their exact layout.
 * Lease release remains valid after metrics invalidation or view removal.
 */
typedef struct ViemRenderRunHandleV1 {
  uint64_t owner;
  uint64_t identifier;
  uint64_t metrics_generation;
  uint32_t threading;
  uint32_t reserved;
} ViemRenderRunHandleV1;

typedef struct ViemClusterCaretStopV1 {
  uint64_t text_offset;
  float inline_offset;
  uint32_t affinity;
} ViemClusterCaretStopV1;

typedef struct ViemShapedClusterV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  float advance;
  ViemTextMetricsV1 metrics;
  ViemShapedBoundsV1 typographic_bounds;
  ViemShapedBoundsV1 ink_bounds;
  uint32_t bidi_level;
  uint32_t has_render_run;
  ViemUtf8Slice fallback_font;
  const ViemClusterCaretStopV1 *caret_stops;
  uint64_t caret_stop_count;
  ViemRenderRunHandleV1 render_run;
} ViemShapedClusterV1;

#define VIEM_SHAPED_CLUSTER_V1_SIZE ((uint32_t)sizeof(ViemShapedClusterV1))

typedef struct ViemShapingDiagnosticV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  ViemUtf8Slice message;
} ViemShapingDiagnosticV1;

#define VIEM_SHAPING_DIAGNOSTIC_V1_SIZE \
  ((uint32_t)sizeof(ViemShapingDiagnosticV1))

/* Passive source metadata. Never fetch an image destination. Local decoding is
 * native-owned and bounded; unavailable and remote images use a 300 x 64 point
 * placeholder. Return one cluster per object with intrinsic dimensions scaled
 * by request.scale, bottom baseline, and only endpoint caret stops. Core clamps
 * the cluster to the content width while preserving aspect ratio. Draw native
 * image resources into the final exported cluster typographic bounds. */
typedef struct ViemInlineImageV1 {
  uint64_t text_start;
  uint64_t text_end;
  ViemUtf8Slice destination;
} ViemInlineImageV1;

typedef struct ViemShapeRequestV1 {
  uint32_t struct_size;
  uint32_t purpose;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  ViemUtf8Slice text;
  ViemUtf8Slice context_before;
  ViemUtf8Slice context_after;
  const ViemShapeStyleRunV1 *style_runs;
  uint64_t style_run_count;
  ViemResolvedTextStyleV1 default_style;
  float scale;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  /*
   * The containing paragraph's VIEM_TEXT_DIRECTION_* value. The provider
   * treats text_start..text_end as a stable ownership interior:
   * shape context_before + text + context_after, then return every whole
   * cluster whose logical start lies in the interior. Such a cluster may end
   * in context_after; omit one whose start lies in context_before. The context
   * slices contain immediately adjacent complete grapheme sequences.
   */
  uint32_t paragraph_base_direction;
  const ViemInlineImageV1 *inline_images;
  uint64_t inline_image_count;
} ViemShapeRequestV1;

#define VIEM_SHAPE_REQUEST_V1_SIZE ((uint32_t)sizeof(ViemShapeRequestV1))

typedef struct ViemShapeResponseV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  const ViemShapedClusterV1 *clusters;
  uint64_t cluster_count;
  const uint64_t *visual_order;
  uint64_t visual_order_count;
  ViemTextMetricsV1 default_metrics;
  const ViemShapingDiagnosticV1 *diagnostics;
  uint64_t diagnostic_count;
} ViemShapeResponseV1;

/*
 * A response echoes the request's text_start..text_end ownership interior.
 * Under provider ABI v4, returned cluster ends may extend into context_after;
 * response bounds do not expand to include those cluster tails.
 */

#define VIEM_SHAPE_RESPONSE_V1_SIZE ((uint32_t)sizeof(ViemShapeResponseV1))

/* One retained fragment lease. Resources stay valid until release, or a
 * metrics-generation change. Response resources are pinned until the next
 * shape_batch call. retain does not invalidate response storage. Release may
 * run on any thread and after view detach: marshal native destruction onto
 * the required executor, and do not call back into core. Both callbacks are
 * required when has_render_run_policy is true. NULL from retain is failure. */
typedef void *(*ViemRetainRenderRunsCallback)(
    void *context, const ViemRenderRunHandleV1 *handles, uint64_t count);
typedef void (*ViemReleaseRenderRunsCallback)(void *lease);

typedef uint64_t (*ViemMetricsGenerationCallback)(void *context);
typedef uint32_t (*ViemShapeBatchCallback)(
    void *context, const ViemShapeRequestV1 *requests, uint64_t request_count,
    ViemShapeResponseV1 *responses, uint64_t response_capacity);

/*
 * The provider table is copied by viem_core_view_add. Only the current
 * provider ABI version is accepted. Context and callback functions remain
 * valid until the view is removed or its core is
 * successfully destroyed. VIEM_STATUS_CORE_BUSY means destruction did not
 * occur and does not end these lifetimes. Every response pointer returned by
 * shape_batch must remain readable until the next shape_batch call for that
 * view; core copies all values immediately. Retaining a fragment lease does
 * not invalidate response storage. Opaque render-run tokens follow the lease
 * and generation lifetime documented on ViemRenderRunHandleV1.
 *
 * A successful ABI-v4 response affirms that its ownership interior is stable
 * under arbitrary text outside the supplied bounded context. A provider that
 * cannot make that guarantee returns VIEM_STATUS_UNSTABLE_SHAPING_CONTEXT from
 * shape_batch instead of returning partial or uncacheable measurements. Core
 * then installs and caches nothing from the failed batch.
 */
typedef struct ViemTextMeasurementProviderV1 {
  uint32_t struct_size;
  uint32_t abi_version;
  void *context;
  uint64_t measurement_environment_id;
  uint32_t threading;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  uint32_t reserved;
  ViemMetricsGenerationCallback metrics_generation;
  ViemShapeBatchCallback shape_batch;
  ViemRetainRenderRunsCallback retain_render_runs;
  ViemReleaseRenderRunsCallback release_render_runs;
} ViemTextMeasurementProviderV1;

#define VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE \
  ((uint32_t)sizeof(ViemTextMeasurementProviderV1))

typedef struct ViemViewOptionsV1 {
  uint32_t struct_size;
  uint32_t execution_context;
  float width;
  float height;
  /* Application canvas padding, applied before the first layout. */
  float padding_top;
  float padding_left;
  float padding_bottom;
  float padding_right;
} ViemViewOptionsV1;

#define VIEM_VIEW_OPTIONS_V1_SIZE ((uint32_t)sizeof(ViemViewOptionsV1))

/*
 * Absolute presentation origin. left is always requested. With HAS_TOP, all
 * expected_* fields must match the current immutable layout. Core maps top
 * through its compact exact/estimated height index, lays out only the local
 * viewport plus bounded overscan, and publishes both coordinates atomically.
 * The expected_* identity is ignored for a horizontal-only request.
 */
typedef struct ViemViewportOriginV1 {
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
} ViemViewportOriginV1;

#define VIEM_VIEWPORT_ORIGIN_V1_SIZE \
  ((uint32_t)sizeof(ViemViewportOriginV1))
#define VIEM_VIEWPORT_ORIGIN_HAS_TOP (1u << 0)

#define VIEM_VIEWPORT_STATE_WRAP (1u << 0)
#define VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT (1u << 1)
#define VIEM_VIEWPORT_STATE_TOP_EXACT (1u << 2)
#define VIEM_VIEWPORT_STATE_HAS_LAYOUT (1u << 3)
/* Reserved compatibility flag, always set: wrapping uses word boundaries. */
#define VIEM_VIEWPORT_STATE_LINEBREAK (1u << 4)
#define VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT (1u << 5)

/* Explicit replacement recovery hints in zero-based logical hard-line and
 * grapheme-column coordinates. These are not snapshot editing positions. */
typedef struct ViemViewRestorationV1 {
  uint32_t struct_size;
  uint32_t cursor_affinity;
  uint64_t cursor_line;
  uint64_t cursor_column;
  uint64_t viewport_line;
  uint64_t viewport_column;
  float row_fraction;
  float left;
} ViemViewRestorationV1;
#define VIEM_VIEW_RESTORATION_V1_SIZE \
  ((uint32_t)sizeof(ViemViewRestorationV1))

/*
 * maximum_left describes rows intersecting the current vertical viewport.
 * It is authoritative only with MAXIMUM_LEFT_EXACT; otherwise it is a
 * provisional visible lower bound, not an upper clamp. scale is the
 * exact positive view-local magnification used by the current configuration.
 * maximum_top includes document padding and final-row geometry. With
 * MAXIMUM_TOP_EXACT it is the document-end clamp in the current coordinate
 * system, even when prefix heights remain estimated. Otherwise it is a
 * scrollbar estimate, not an authoritative clamp.
 * A missing TOP_EXACT flag means the current top depends on estimated prefix
 * heights; it remains presentation state but is not an exact absolute
 * document y. The dependency identity is captured atomically with these flags
 * and values.
 */
typedef struct ViemViewportStateV1 {
  uint32_t struct_size;
  uint32_t flags;
  float left;
  float top;
  float maximum_left;
  float maximum_top;
  float scale;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} ViemViewportStateV1;

#define VIEM_VIEWPORT_STATE_V1_SIZE \
  ((uint32_t)sizeof(ViemViewportStateV1))

/*
 * Exact identity for one immutable layout snapshot. Copy this value from a
 * snapshot-info query into every dependent geometry request. Core never
 * silently substitutes a newer layout.
 */
typedef struct ViemLayoutSnapshotIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} ViemLayoutSnapshotIdentityV1;

#define VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutSnapshotIdentityV1))

typedef struct ViemLayoutInsetsV1 {
  float top;
  float left;
  float bottom;
  float right;
} ViemLayoutInsetsV1;

typedef struct ViemLayoutRectV1 {
  float x;
  float y;
  float width;
  float height;
} ViemLayoutRectV1;

/* Normalized RGBA components; each value is finite and in [0, 1]. */
typedef struct ViemRgbaV1 {
  float red;
  float green;
  float blue;
  float alpha;
} ViemRgbaV1;

#define VIEM_STYLE_NAMESPACE_BLOCK 1u
#define VIEM_STYLE_NAMESPACE_CHARACTER 2u

#define VIEM_STYLE_ROLE_NONE 0u
#define VIEM_STYLE_ROLE_DOCUMENT 1u
#define VIEM_STYLE_ROLE_PARAGRAPH 2u
#define VIEM_STYLE_ROLE_QUOTE 3u
#define VIEM_STYLE_ROLE_CODE_BLOCK 4u
#define VIEM_STYLE_ROLE_LIST 5u
#define VIEM_STYLE_ROLE_LIST_ITEM 6u
#define VIEM_STYLE_ROLE_TABLE 7u


#define VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION 2u
#define VIEM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY 3u

#define VIEM_STYLE_DEFINITION_HAS_PARENT (1u << 0)
#define VIEM_STYLE_DEFINITION_HAS_NEXT_STYLE (1u << 1)
#define VIEM_STYLE_DEFINITION_BASE_PARAGRAPH (1u << 3)
#define VIEM_STYLE_DEFINITION_INTERNAL (1u << 5)
#define VIEM_STYLE_DEFINITION_INTERNAL_LIST (1u << 6)
/* A generated Code syntax definition that has not been edited or persisted. */
#define VIEM_STYLE_DEFINITION_IMPLICIT (1u << 7)

#define VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS (1u << 0)
#define VIEM_STYLE_CAPABILITY_EDIT_PARENT (1u << 1)
#define VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE (1u << 2)
#define VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME (1u << 3)
#define VIEM_STYLE_CAPABILITY_ASSIGN (1u << 4)
#define VIEM_STYLE_CAPABILITY_DELETE (1u << 5)

#define VIEM_STYLE_PROPERTY_CANVAS_BACKGROUND 1u
#define VIEM_STYLE_PROPERTY_CANVAS_PADDING_TOP 2u
#define VIEM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT 3u
#define VIEM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM 4u
#define VIEM_STYLE_PROPERTY_CANVAS_PADDING_LEFT 5u
#define VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP 6u
#define VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM 7u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING 8u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT 9u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT 10u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT 11u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT 12u
#define VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION 13u
#define VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES 14u
#define VIEM_STYLE_PROPERTY_CHARACTER_SIZE 15u
#define VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT 16u
#define VIEM_STYLE_PROPERTY_CHARACTER_SLANT 17u
#define VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND 18u
#define VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND 19u
#define VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE 20u
#define VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH 21u
#define VIEM_STYLE_PROPERTY_CHARACTER_LANGUAGE 22u
#define VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION 23u
#define VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES 24u
#define VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING 25u
#define VIEM_STYLE_PROPERTY_CHARACTER_BOLD 27u
#define VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT 28u
#define VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT 29u
#define VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP 30u
#define VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT 31u
#define VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM 32u
#define VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT 33u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH 34u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR 35u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH 36u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR 37u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH 38u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR 39u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH 40u
#define VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR 41u
#define VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND 42u
#define VIEM_STYLE_PROPERTY_CHARACTER_FONT_AXES 43u
#define VIEM_STYLE_PROPERTY_CHARACTER_FONT_FACE 44u


#define VIEM_STYLE_VALUE_NONE 0u
#define VIEM_STYLE_VALUE_FLOAT 1u
#define VIEM_STYLE_VALUE_UNSIGNED 2u
#define VIEM_STYLE_VALUE_BOOLEAN 3u
#define VIEM_STYLE_VALUE_COLOR 4u
#define VIEM_STYLE_VALUE_STRING 5u
#define VIEM_STYLE_VALUE_STRING_LIST 6u
#define VIEM_STYLE_VALUE_FONT_SLANT 7u
#define VIEM_STYLE_VALUE_WRITING_DIRECTION 8u
#define VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES 9u
#define VIEM_STYLE_VALUE_LINE_SPACING 10u
#define VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT 11u
/* Named-style CharacterSize declarations only: enum_value is an integer 10..1000;
 * number is zero. Effective CharacterSize values remain FLOAT points. */
#define VIEM_STYLE_VALUE_PERCENTAGE 13u

#define VIEM_STYLE_VALUE_ITEM_STRING 1u
#define VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE 2u

#define VIEM_STYLE_LINE_SPACING_NORMAL 1u
#define VIEM_STYLE_LINE_SPACING_MULTIPLIER 2u
#define VIEM_STYLE_LINE_SPACING_AT_LEAST 3u
#define VIEM_STYLE_LINE_SPACING_EXACT 4u

#define VIEM_STYLE_PARAGRAPH_ALIGNMENT_START 1u
#define VIEM_STYLE_PARAGRAPH_ALIGNMENT_END 2u
#define VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER 3u

#define VIEM_STYLE_PROPERTY_DECLARED (1u << 0)
#define VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT (1u << 1)
#define VIEM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE (1u << 2)
/* Selection-format snapshots only: selected runs disagree on this property. */

#define VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY 1u
#define VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE 2u
#define VIEM_STYLE_CONTRIBUTOR_CHARACTER_STYLE 3u
#define VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS 4u
#define VIEM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER 5u
#define VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH 6u
#define VIEM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER 7u
#define VIEM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER 8u

#define VIEM_STYLE_EDIT_SET_DECLARATION 1u
#define VIEM_STYLE_EDIT_CLEAR_DECLARATION 2u
#define VIEM_STYLE_EDIT_SET_PARENT 3u
#define VIEM_STYLE_EDIT_CLEAR_PARENT 4u
#define VIEM_STYLE_EDIT_SET_NEXT_STYLE 5u
#define VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE 6u
#define VIEM_STYLE_EDIT_SET_DISPLAY_NAME 7u

typedef struct ViemStyleSheetIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
} ViemStyleSheetIdentityV1;

#define VIEM_STYLE_SHEET_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleSheetIdentityV1))

typedef struct ViemStyleStringRefV1 {
  uint64_t offset;
  uint64_t length;
} ViemStyleStringRefV1;

typedef struct ViemStyleSheetInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemStyleSheetIdentityV1 identity;
  uint64_t definition_count;
  uint64_t property_count;
  uint64_t value_item_count;
  uint64_t dependency_count;
  uint64_t string_bytes;
} ViemStyleSheetInfoV1;

#define VIEM_STYLE_SHEET_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleSheetInfoV1))

typedef struct ViemStyleValueV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t enum_value;
  uint32_t reserved;
  float number;
  float number_reserved;
  ViemRgbaV1 color;
  ViemStyleStringRefV1 string;
  uint64_t first_item;
  uint64_t item_count;
} ViemStyleValueV1;

#define VIEM_STYLE_VALUE_V1_SIZE ((uint32_t)sizeof(ViemStyleValueV1))

typedef struct ViemStyleValueItemV1 {
  uint32_t struct_size;
  uint32_t kind;
  ViemStyleStringRefV1 string;
  uint32_t unsigned_value;
  uint32_t reserved;
} ViemStyleValueItemV1;

#define VIEM_STYLE_VALUE_ITEM_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleValueItemV1))

typedef struct ViemStyleDependencyV1 {
  uint32_t struct_size;
  uint32_t namespace_id;
  ViemStyleStringRefV1 style_id;
} ViemStyleDependencyV1;

#define VIEM_STYLE_DEPENDENCY_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleDependencyV1))

typedef struct ViemStyleDefinitionV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t namespace_id;
  uint32_t role;
  uint32_t origin;
  uint32_t capabilities;
  ViemStyleStringRefV1 stable_id;
  ViemStyleStringRefV1 display_name;
  ViemStyleStringRefV1 parent_id;
  ViemStyleStringRefV1 next_style_id;
  uint64_t first_property;
  uint64_t property_count;
} ViemStyleDefinitionV1;

#define VIEM_STYLE_DEFINITION_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleDefinitionV1))

typedef struct ViemStylePropertyV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t property;
  uint32_t contributor_kind;
  uint32_t contributor_namespace;
  uint32_t reserved;
  ViemStyleValueV1 declared;
  ViemStyleValueV1 effective;
  ViemStyleStringRefV1 contributor_style_id;
  uint64_t first_dependency;
  uint64_t dependency_count;
} ViemStylePropertyV1;

#define VIEM_STYLE_PROPERTY_V1_SIZE \
  ((uint32_t)sizeof(ViemStylePropertyV1))

typedef struct ViemStyleEditValueItemV1 {
  uint32_t struct_size;
  uint32_t kind;
  ViemUtf8Slice text;
  uint32_t unsigned_value;
  uint32_t reserved;
} ViemStyleEditValueItemV1;

#define VIEM_STYLE_EDIT_VALUE_ITEM_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleEditValueItemV1))

typedef struct ViemStyleEditValueV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t enum_value;
  uint32_t reserved;
  float number;
  float number_reserved;
  ViemRgbaV1 color;
  ViemUtf8Slice text;
  const ViemStyleEditValueItemV1 *items;
  uint64_t item_count;
} ViemStyleEditValueV1;

#define VIEM_STYLE_EDIT_VALUE_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleEditValueV1))

typedef struct ViemStyleEditV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemStyleSheetIdentityV1 identity;
  uint32_t namespace_id;
  uint32_t operation;
  uint32_t property;
  uint32_t reserved;
  ViemUtf8Slice style_id;
  ViemStyleEditValueV1 value;
} ViemStyleEditV1;

#define VIEM_STYLE_EDIT_V1_SIZE ((uint32_t)sizeof(ViemStyleEditV1))

/*
 * Immutable capability for one explicit live style-edit group. token is
 * process-wide and non-reused. Every other field is also part of the
 * capability and must be returned unchanged. Begin revisions remain fixed as
 * grouped edits commit newer snapshots.
 */
typedef struct ViemStyleEditGroupV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t token;
  ViemViewId view_id;
  uint64_t document_id;
  uint64_t begin_document_revision;
  uint64_t begin_style_sheet_revision;
} ViemStyleEditGroupV1;

#define VIEM_STYLE_EDIT_GROUP_V1_SIZE \
  ((uint32_t)sizeof(ViemStyleEditGroupV1))

#define VIEM_TEXT_PAINT_HAS_BACKGROUND (1u << 0)
#define VIEM_TEXT_PAINT_UNDERLINE (1u << 1)
#define VIEM_TEXT_PAINT_STRIKETHROUGH (1u << 2)
#define VIEM_TEXT_PAINT_DEFAULT_FOREGROUND (1u << 3)
#define VIEM_LAYOUT_PAINT_DEFAULT_CANVAS (1u << 0)

/*
 * Foreground is always present. Background is meaningful only with
 * HAS_BACKGROUND. Decoration flags mean that decoration is enabled.
 */
typedef struct ViemTextPaintV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemRgbaV1 foreground;
  ViemRgbaV1 background;
} ViemTextPaintV1;

#define VIEM_TEXT_PAINT_V1_SIZE ((uint32_t)sizeof(ViemTextPaintV1))

/* Canvas/default paint and required override-run count for one exact layout. */
typedef struct ViemLayoutPaintInfoV1 {
  uint32_t struct_size;
  /* Formerly reserved; output flags preserve the v1 binary layout. */
  uint32_t flags;
  ViemLayoutSnapshotIdentityV1 identity;
  ViemRgbaV1 canvas_background;
  ViemTextPaintV1 default_paint;
  uint64_t paint_run_count;
} ViemLayoutPaintInfoV1;

#define VIEM_LAYOUT_PAINT_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutPaintInfoV1))

/* Ordered, non-overlapping logical half-open UTF-8 override range. */
typedef struct ViemPaintStyleRunV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  ViemTextPaintV1 paint;
} ViemPaintStyleRunV1;

#define VIEM_PAINT_STYLE_RUN_V1_SIZE \
  ((uint32_t)sizeof(ViemPaintStyleRunV1))

#define VIEM_LAYOUT_SNAPSHOT_FULL_DOCUMENT (1u << 0)
#define VIEM_LAYOUT_SNAPSHOT_PREFIX_EXACT (1u << 1)
#define VIEM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT (1u << 2)
#define VIEM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT (1u << 3)

/* Geometry is in document-layout coordinates; subtract the viewport origin. */
typedef struct ViemLayoutSnapshotInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemLayoutSnapshotIdentityV1 identity;
  float viewport_width;
  float viewport_height;
  float usable_width;
  float content_width;
  float total_height;
  ViemLayoutInsetsV1 content_insets;
  uint64_t coverage_hard_line_start;
  uint64_t coverage_hard_line_end;
  uint64_t document_hard_line_count;
  float coverage_y_start;
  float coverage_y_end;
  uint64_t row_count;
  uint64_t cluster_count;
  uint64_t caret_count;
} ViemLayoutSnapshotInfoV1;

#define VIEM_LAYOUT_SNAPSHOT_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutSnapshotInfoV1))

#define VIEM_VISUAL_ROW_HAS_PARAGRAPH (1u << 0)
#define VIEM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS (1u << 1)
#define VIEM_VISUAL_ROW_WRAPS_TO_NEXT (1u << 2)

/* first_* and *_count index arrays copied by the same snapshot export. */
typedef struct ViemVisualRowV1 {
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
} ViemVisualRowV1;

#define VIEM_VISUAL_ROW_V1_SIZE ((uint32_t)sizeof(ViemVisualRowV1))

#define VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN (1u << 0)

typedef struct ViemPositionedClusterV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t row_index;
  uint64_t text_start;
  uint64_t text_end;
  float x;
  float advance;
  ViemLayoutRectV1 typographic_bounds;
  ViemLayoutRectV1 ink_bounds;
  uint32_t bidi_level;
  uint32_t reserved;
  ViemRenderRunHandleV1 render_run;
} ViemPositionedClusterV1;

#define VIEM_POSITIONED_CLUSTER_V1_SIZE \
  ((uint32_t)sizeof(ViemPositionedClusterV1))

#define VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER (1u << 1)
#define VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND (1u << 2)
#define VIEM_LAYOUT_DECORATION_BLOCK_BORDER (1u << 3)
/* Noneditable block furniture; label offsets address only the separate label
 * byte blob. No decoration creates formatted offsets, caret or selection stops. */
typedef struct ViemLayoutDecorationV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t row_index;
  uint64_t label_byte_start;
  uint64_t label_byte_length;
  float x;
  float advance;
  float font_size;
  float reserved;
  ViemLayoutRectV1 typographic_bounds;
  ViemLayoutRectV1 ink_bounds;
  ViemRenderRunHandleV1 render_run;
  ViemTextPaintV1 paint;
} ViemLayoutDecorationV1;
#define VIEM_LAYOUT_DECORATION_V1_SIZE ((uint32_t)sizeof(ViemLayoutDecorationV1))
typedef struct ViemLayoutDecorationsInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemLayoutSnapshotIdentityV1 identity;
  uint64_t decoration_count;
  uint64_t label_bytes;
} ViemLayoutDecorationsInfoV1;
#define VIEM_LAYOUT_DECORATIONS_INFO_V1_SIZE ((uint32_t)sizeof(ViemLayoutDecorationsInfoV1))

typedef struct ViemPositionedCaretV1 {
  uint32_t struct_size;
  uint32_t affinity;
  uint64_t row_index;
  uint64_t text_offset;
  float x;
  float reserved;
} ViemPositionedCaretV1;

#define VIEM_POSITIONED_CARET_V1_SIZE \
  ((uint32_t)sizeof(ViemPositionedCaretV1))

typedef struct ViemLayoutCaretRequestV1 {
  uint32_t struct_size;
  uint32_t affinity;
  ViemLayoutSnapshotIdentityV1 identity;
  uint64_t text_offset;
} ViemLayoutCaretRequestV1;

#define VIEM_LAYOUT_CARET_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutCaretRequestV1))

/* Character-cell hit testing for pointer-down in character-addressing modes. */
#define VIEM_LAYOUT_HIT_TEST_POINTER_DOWN (1u << 0)

typedef struct ViemLayoutHitTestRequestV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemLayoutSnapshotIdentityV1 identity;
  float x;
  float y;
} ViemLayoutHitTestRequestV1;

#define VIEM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutHitTestRequestV1))

typedef struct ViemLayoutCaretPointV1 {
  uint32_t struct_size;
  uint32_t affinity;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t text_offset;
} ViemLayoutCaretPointV1;

#define VIEM_LAYOUT_CARET_POINT_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutCaretPointV1))

#define VIEM_CARET_GEOMETRY_CLUSTER_FALLBACK (1u << 0)

typedef struct ViemLayoutCaretGeometryV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemLayoutCaretPointV1 point;
  ViemLayoutRectV1 rect;
  uint64_t row_index;
} ViemLayoutCaretGeometryV1;

#define VIEM_LAYOUT_CARET_GEOMETRY_V1_SIZE \
  ((uint32_t)sizeof(ViemLayoutCaretGeometryV1))

/* Predicted plain-click caret; CURRENT means it already occupies this target. */
#define VIEM_POINTER_CARET_CURRENT (1u << 0)
typedef struct ViemPointerCaretV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t mode;
  uint32_t caret_shape;
  uint32_t affinity;
  float font_en_width;
  uint64_t text_start;
  uint64_t text_end;
} ViemPointerCaretV1;
#define VIEM_POINTER_CARET_V1_SIZE ((uint32_t)sizeof(ViemPointerCaretV1))

/* Read-only plain-click prediction. Request flags must be zero. */
ViemStatus viem_core_view_pointer_caret(ViemCoreHandle handle, ViemViewId view,
    const ViemLayoutHitTestRequestV1 *request, ViemPointerCaretV1 *out_caret);

#define VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR (1u << 0)
#define VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT (1u << 1)
#define VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK (1u << 2)
#define VIEM_VIEW_PRESENTATION_HAS_COMMAND_LINE (1u << 3)
#define VIEM_VIEW_PRESENTATION_HAS_DESIRED_X (1u << 4)
/* Route the next input to the core before native editing shortcuts. */
#define VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING (1u << 5)
/* Route prompt register selectors to core, including native text events. */
#define VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING (1u << 6)

/*
 * Linear Visual anchors have no retained visual affinity, so the anchor
 * affinity is zero unless VISUAL_ANCHOR_AFFINITY_EXACT is present. Visual
 * Block endpoints retain exact affinity and display-space x edges.
 */
typedef struct ViemViewPresentationV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint32_t mode;
  /* Row disambiguation for caret_utf8_start when caret_shape is a boundary.
     Never use it to choose which character a cell caret covers. */
  uint32_t cursor_affinity;
  uint32_t visual_anchor_affinity;
  /* VIEM_CARET_SHAPE_CELL or VIEM_CARET_SHAPE_BOUNDARY. */
  uint32_t caret_shape;
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
  /* Exact formatted range the caret occupies. A cell covers one grapheme of
     hard-line content; a boundary is empty, with both ends at the caret. */
  uint64_t caret_utf8_start;
  uint64_t caret_utf8_end;
} ViemViewPresentationV1;

#define VIEM_VIEW_PRESENTATION_V1_SIZE \
  ((uint32_t)sizeof(ViemViewPresentationV1))

/* The caret covers one grapheme: draw the cell caret_utf8_start..caret_utf8_end.
   Boundary affinity does not apply. */
#define VIEM_CARET_SHAPE_CELL 1u
/* The caret sits between graphemes at caret_utf8_start; cursor_affinity picks
   its visual row at a soft-wrap boundary. */
#define VIEM_CARET_SHAPE_BOUNDARY 2u

#define VIEM_COMMAND_LINE_KIND_NONE 0u
#define VIEM_COMMAND_LINE_KIND_EX 1u
#define VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD 2u
#define VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD 3u

/*
 * state_identity is an opaque exact-content token. It changes whenever the
 * exported kind, UTF-8 cursor, or bytes change without a document revision.
 */
typedef struct ViemCommandLineIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint8_t state_identity[32];
} ViemCommandLineIdentityV1;

#define VIEM_COMMAND_LINE_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemCommandLineIdentityV1))

/* The prompt prefix is represented by identity.kind, not by the UTF-8 bytes. */
typedef struct ViemCommandLineInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemCommandLineIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t cursor_utf8_offset;
} ViemCommandLineInfoV1;

#define VIEM_COMMAND_LINE_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemCommandLineInfoV1))

#define VIEM_COMPLETION_ACTIVE (1u << 0)
#define VIEM_COMPLETION_SEARCHING (1u << 1)
#define VIEM_COMPLETION_TRUNCATED (1u << 2)
#define VIEM_COMPLETION_HAS_ANCHOR (1u << 3)
#define VIEM_COMPLETION_RIGHT_TO_LEFT (1u << 4)

/*
 * Backend-owned Insert completion presentation. The session and generation
 * identify one exact item ordering and selected index. Inactive completion has
 * zero flags, session, generation, and counts. selected_index is -1 for the
 * original text; otherwise it indexes the item array. The frontend displays
 * these values and continues forwarding ordinary normalized input to the core.
 * HAS_ANCHOR supplies the completed word's logical leading edge in exact
 * anchor_layout coordinates. Align popup text there, accounting for native
 * padding. RIGHT_TO_LEFT mirrors the popup and aligns the text's right edge.
 * No anchor is published until prefix discovery and exact geometry are ready.
 */
typedef struct ViemCompletionInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t session_id;
  uint64_t generation;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t view_id;
  int64_t selected_index;
  uint64_t item_count;
  uint64_t utf8_length;
  ViemLayoutSnapshotIdentityV1 anchor_layout;
  ViemLayoutRectV1 anchor_rect;
} ViemCompletionInfoV1;

#define VIEM_COMPLETION_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemCompletionInfoV1))

/* Byte ranges in the concatenated, length-delimited UTF-8 candidate arena. */
typedef struct ViemCompletionItemV1 {
  uint64_t text_offset;
  uint64_t text_length;
} ViemCompletionItemV1;

#define VIEM_COMPLETION_ITEM_V1_SIZE \
  ((uint32_t)sizeof(ViemCompletionItemV1))

#define VIEM_VISUAL_SELECTION_KIND_NONE 0u
#define VIEM_VISUAL_SELECTION_KIND_CHARACTER 1u
#define VIEM_VISUAL_SELECTION_KIND_LINE 2u
#define VIEM_VISUAL_SELECTION_KIND_BLOCK 3u

#define VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW (1u << 0)
#define VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE (1u << 1)
#define VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES (1u << 2)

/*
 * layout identifies the exact immutable geometry. state_identity is an opaque
 * exact-payload token that also changes when selection state changes without
 * relayout.
 */
typedef struct ViemVisualSelectionIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  ViemLayoutSnapshotIdentityV1 layout;
  uint8_t state_identity[32];
} ViemVisualSelectionIdentityV1;

#define VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemVisualSelectionIdentityV1))

/*
 * Segments are in logical UTF-8 document order. Character/Line segments retain
 * the entire selected range, including text outside materialized layout.
 * Rectangles cover only materialized geometry, in visual-row, then x order.
 */
typedef struct ViemVisualSelectionInfoV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemVisualSelectionIdentityV1 identity;
  uint64_t segment_count;
  uint64_t rectangle_count;
} ViemVisualSelectionInfoV1;

#define VIEM_VISUAL_SELECTION_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemVisualSelectionInfoV1))

/*
 * One logical half-open UTF-8 range. Character/Line segments have zero flags.
 * Block segments retain their visual row, hard line, and display-left/right
 * affinities; these affinities are not logical start/end labels in bidi text.
 */
typedef struct ViemVisualSelectionSegmentV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t text_start;
  uint64_t text_end;
  uint64_t row_index;
  uint64_t hard_line_index;
  uint32_t left_affinity;
  uint32_t right_affinity;
} ViemVisualSelectionSegmentV1;

#define VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE \
  ((uint32_t)sizeof(ViemVisualSelectionSegmentV1))

/* segment_index indexes the segment array copied by the same call. */
typedef struct ViemVisualSelectionRectangleV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t row_index;
  uint64_t segment_index;
  ViemLayoutRectV1 rect;
} ViemVisualSelectionRectangleV1;

#define VIEM_VISUAL_SELECTION_RECTANGLE_V1_SIZE \
  ((uint32_t)sizeof(ViemVisualSelectionRectangleV1))

#define VIEM_LOGICAL_SELECTION_KIND_NONE 0u
#define VIEM_LOGICAL_SELECTION_KIND_CHARACTER 1u
#define VIEM_LOGICAL_SELECTION_KIND_LINE 2u
#define VIEM_LOGICAL_SELECTION_KIND_BLOCK 3u
#define VIEM_LOGICAL_SELECTION_KIND_CELLS 4u

#define VIEM_SEMANTIC_STYLE_STRONG 1u
#define VIEM_SEMANTIC_STYLE_EMPHASIS 2u

#define VIEM_SEMANTIC_STYLE_STATE_OFF 0u
#define VIEM_SEMANTIC_STYLE_STATE_ON 1u
#define VIEM_SEMANTIC_STYLE_STATE_MIXED 2u

#define VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE (1u << 0)
#define VIEM_SEMANTIC_STYLE_CAN_SET (1u << 1)
#define VIEM_SEMANTIC_STYLE_CAN_CLEAR (1u << 2)
/* Insert/Replace caret; changes pending typing declarations, not source. */
#define VIEM_SEMANTIC_STYLE_TYPING_CONTEXT (1u << 3)

/*
 * Exact logical selection identity with no layout dependency. Only Character
 * and Line selections name an actionable contiguous range in this ABI.
 */
typedef struct ViemLogicalSelectionIdentityV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t text_start;
  uint64_t text_end;
  uint8_t state_identity[32];
} ViemLogicalSelectionIdentityV1;

#define VIEM_LOGICAL_SELECTION_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemLogicalSelectionIdentityV1))

#define VIEM_TABLE_CAN_INSERT 1u
#define VIEM_TABLE_IN_TABLE 2u
#define VIEM_TABLE_HEADER 4u
#define VIEM_TABLE_CELL_SELECTED 8u
#define VIEM_TABLE_ALIGN_UNSPECIFIED 0u
#define VIEM_TABLE_ALIGN_LEFT 1u
#define VIEM_TABLE_ALIGN_CENTER 2u
#define VIEM_TABLE_ALIGN_RIGHT 3u
#define VIEM_TABLE_INSERT_ROW_ABOVE 1u
#define VIEM_TABLE_INSERT_ROW_BELOW 2u
#define VIEM_TABLE_DELETE_ROW 3u
#define VIEM_TABLE_INSERT_COLUMN_LEFT 4u
#define VIEM_TABLE_INSERT_COLUMN_RIGHT 5u
#define VIEM_TABLE_DELETE_COLUMN 6u
#define VIEM_TABLE_SET_ALIGNMENT 7u
#define VIEM_TABLE_CLEAR_CELL 8u

typedef struct ViemTableContextV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemLogicalSelectionIdentityV1 selection;
  uint64_t table_id;
  uint64_t row;
  uint64_t column;
  uint64_t rows;
  uint64_t columns;
  uint32_t alignment;
  uint32_t reserved;
} ViemTableContextV1;
#define VIEM_TABLE_CONTEXT_V1_SIZE ((uint32_t)sizeof(ViemTableContextV1))

typedef struct ViemInsertTableV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemLogicalSelectionIdentityV1 expected_selection;
  uint32_t columns;
  uint32_t body_rows;
} ViemInsertTableV1;
#define VIEM_INSERT_TABLE_V1_SIZE ((uint32_t)sizeof(ViemInsertTableV1))

typedef struct ViemTableActionV1 {
  uint32_t struct_size;
  uint32_t action;
  ViemTableContextV1 expected;
  uint32_t alignment;
  uint32_t reserved;
} ViemTableActionV1;
#define VIEM_TABLE_ACTION_V1_SIZE ((uint32_t)sizeof(ViemTableActionV1))

/* Current WYSIWYG cell geometry; rects are in document layout coordinates.
 * Exported only from the exact requested immutable layout snapshot. */
typedef struct ViemTableCellV1 {
  uint64_t table_id;
  uint64_t row;
  uint64_t column;
  uint64_t cell_id;
  uint64_t text_start;
  uint64_t text_end;
  ViemLayoutRectV1 rect;
  ViemLayoutRectV1 table_rect;
  uint32_t alignment;
  uint32_t flags;
} ViemTableCellV1;
#define VIEM_TABLE_CELL_V1_SIZE ((uint32_t)sizeof(ViemTableCellV1))

typedef struct ViemTableSelectionV1 {
  uint32_t struct_size;
  uint32_t active;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t table_id;
  uint64_t anchor_row;
  uint64_t anchor_column;
  uint64_t active_row;
  uint64_t active_column;
} ViemTableSelectionV1;
#define VIEM_TABLE_SELECTION_V1_SIZE ((uint32_t)sizeof(ViemTableSelectionV1))

/* Adapter capability and check/mixed state at one core-owned selection. */
typedef struct ViemSemanticStylePresentationV1 {
  uint32_t struct_size;
  uint32_t style;
  uint32_t state;
  uint32_t flags;
  ViemLogicalSelectionIdentityV1 selection;
} ViemSemanticStylePresentationV1;

#define VIEM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE \
  ((uint32_t)sizeof(ViemSemanticStylePresentationV1))

/* Exact request using an identity returned by the presentation query. */
typedef struct ViemSetSemanticStyleV1 {
  uint32_t struct_size;
  uint32_t style;
  uint32_t enabled;
  uint32_t reserved;
  ViemLogicalSelectionIdentityV1 expected_selection;
} ViemSetSemanticStyleV1;

#define VIEM_SET_SEMANTIC_STYLE_V1_SIZE \
  ((uint32_t)sizeof(ViemSetSemanticStyleV1))

#define VIEM_PLACE_CURSOR_EXTEND_SELECTION (1u << 0)
/* Select whole words; EXTEND retains the gesture's original word. */
#define VIEM_PLACE_CURSOR_WORD_SELECTION (1u << 1)
/* Seed a character-inclusive drag origin; incompatible with extend/word flags. */
#define VIEM_PLACE_CURSOR_BEGIN_POINTER_GESTURE (1u << 2)

typedef struct ViemPlaceCursorV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t document_revision;
  uint64_t text_offset;
  uint32_t affinity;
  uint32_t reserved;
} ViemPlaceCursorV1;

#define VIEM_PLACE_CURSOR_V1_SIZE ((uint32_t)sizeof(ViemPlaceCursorV1))

/* Concrete file-format target bound to one exact document snapshot. */
typedef struct ViemSetFileFormatV1 {
  uint32_t struct_size;
  uint32_t file_format;
  uint64_t document_id;
  uint64_t document_revision;
} ViemSetFileFormatV1;

#define VIEM_SET_FILE_FORMAT_V1_SIZE \
  ((uint32_t)sizeof(ViemSetFileFormatV1))

/* Switch between Markdown Source and WYSIWYG without changing source bytes. */
typedef struct ViemSetMarkdownSourceV1 {
  uint32_t struct_size;
  uint32_t source;
  uint64_t document_id;
  uint64_t document_revision;
} ViemSetMarkdownSourceV1;
#define VIEM_SET_MARKDOWN_SOURCE_V1_SIZE ((uint32_t)sizeof(ViemSetMarkdownSourceV1))

typedef struct ViemSetEncodingV1 {
  uint32_t struct_size;
  uint32_t encoding;
  uint64_t document_id;
  uint64_t document_revision;
} ViemSetEncodingV1;
#define VIEM_SET_ENCODING_V1_SIZE ((uint32_t)sizeof(ViemSetEncodingV1))

#define VIEM_LIST_STYLE_NONE 0u
#define VIEM_LIST_STYLE_BULLET 1u
#define VIEM_LIST_STYLE_NUMBERED 2u
typedef struct ViemSetListStyleV1 {
  uint32_t struct_size;
  uint32_t style;
  ViemLogicalSelectionIdentityV1 expected_selection;
} ViemSetListStyleV1;
#define VIEM_SET_LIST_STYLE_V1_SIZE ((uint32_t)sizeof(ViemSetListStyleV1))

typedef struct ViemSetBlockQuoteV1 {
  uint32_t struct_size;
  uint32_t enabled;
  ViemLogicalSelectionIdentityV1 expected_selection;
} ViemSetBlockQuoteV1;
#define VIEM_SET_BLOCK_QUOTE_V1_SIZE ((uint32_t)sizeof(ViemSetBlockQuoteV1))

#define VIEM_LIST_CAN_INDENT 1u
#define VIEM_LIST_CAN_UNINDENT 2u
typedef struct ViemListIndentV1 {
  uint32_t struct_size;
  uint32_t unindent;
  ViemLogicalSelectionIdentityV1 expected_selection;
} ViemListIndentV1;
#define VIEM_LIST_INDENT_V1_SIZE ((uint32_t)sizeof(ViemListIndentV1))

typedef struct ViemSetParagraphStyleV1 {
  uint32_t struct_size;
  uint32_t level; /* 0 = Base Paragraph, 1..6 = heading. */
  ViemLogicalSelectionIdentityV1 expected_selection;
} ViemSetParagraphStyleV1;
#define VIEM_SET_PARAGRAPH_STYLE_V1_SIZE ((uint32_t)sizeof(ViemSetParagraphStyleV1))

/* Named paragraph or character style assignment. With no selection a character
 * assignment updates subsequent typing. An empty character style_id clears the
 * named assignment (Default Paragraph); it is never a stored style definition.
 * Paragraph assignment accepts the current paragraph identity. */
typedef struct ViemAssignStyleV1 {
  uint32_t struct_size;
  uint32_t namespace;
  ViemStyleSheetIdentityV1 identity;
  ViemLogicalSelectionIdentityV1 expected_selection;
  ViemUtf8Slice style_id;
} ViemAssignStyleV1;
#define VIEM_ASSIGN_STYLE_V1_SIZE ((uint32_t)sizeof(ViemAssignStyleV1))

/* New sparse Code style definition. Empty parent uses the namespace base;
 * empty next style leaves it absent. */
typedef struct ViemCreateStyleV1 {
  uint32_t struct_size;
  uint32_t namespace;
  ViemStyleSheetIdentityV1 identity;
  ViemUtf8Slice style_id;
  ViemUtf8Slice display_name;
  ViemUtf8Slice parent_id;
  ViemUtf8Slice next_style_id;
} ViemCreateStyleV1;
#define VIEM_CREATE_STYLE_V1_SIZE ((uint32_t)sizeof(ViemCreateStyleV1))

typedef struct ViemDeleteStyleV1 {
  uint32_t struct_size;
  uint32_t namespace;
  ViemStyleSheetIdentityV1 identity;
  ViemUtf8Slice style_id;
} ViemDeleteStyleV1;
#define VIEM_DELETE_STYLE_V1_SIZE ((uint32_t)sizeof(ViemDeleteStyleV1))

/* Exact acknowledgement of a successfully persisted native snapshot. */
typedef struct ViemMarkSavedV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
} ViemMarkSavedV1;

#define VIEM_MARK_SAVED_V1_SIZE ((uint32_t)sizeof(ViemMarkSavedV1))

typedef struct ViemKeyInputV1 {
  uint32_t struct_size;
  uint32_t kind;
  /* Unicode scalar, or function-key number 1..35 for VIEM_KEY_FUNCTION. */
  uint32_t codepoint;
  /* VIEM_KEY_MODIFIER_* bits for function and navigation keys; zero otherwise. */
  uint32_t modifiers;
} ViemKeyInputV1;

#define VIEM_KEY_INPUT_V1_SIZE ((uint32_t)sizeof(ViemKeyInputV1))

/*
 * Exact formatted-snapshot replacement target for native marked text. The
 * frontend passes its current selection, or an empty range at its insertion
 * caret, using UTF-8 byte offsets for document_revision.
 */
typedef struct ViemCompositionBeginV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  uint64_t replacement_start;
  uint64_t replacement_end;
} ViemCompositionBeginV1;

#define VIEM_COMPOSITION_BEGIN_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionBeginV1))

/* selected_start/end are UTF-8 byte offsets relative to marked_text. */
typedef struct ViemCompositionUpdateV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  ViemUtf8Slice marked_text;
  uint64_t selected_start;
  uint64_t selected_end;
} ViemCompositionUpdateV1;

#define VIEM_COMPOSITION_UPDATE_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionUpdateV1))

typedef struct ViemCompositionCommitV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  ViemUtf8Slice committed_text;
} ViemCompositionCommitV1;

#define VIEM_COMPOSITION_COMMIT_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionCommitV1))

typedef struct ViemCompositionCancelV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
} ViemCompositionCancelV1;

#define VIEM_COMPOSITION_CANCEL_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionCancelV1))

#define VIEM_COMPOSITION_OVERLAY_ACTIVE (1u << 0)

typedef struct ViemCompositionOverlayIdentityV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t generation;
} ViemCompositionOverlayIdentityV1;

#define VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionOverlayIdentityV1))

/* Exact ranges in the disposable, source-nonmutating composed projection. */
typedef struct ViemCompositionOverlayInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  ViemCompositionOverlayIdentityV1 identity;
  uint64_t utf8_length;
  uint64_t replacement_start;
  uint64_t replacement_end;
  uint64_t marked_start;
  uint64_t marked_end;
  uint64_t selected_start;
  uint64_t selected_end;
} ViemCompositionOverlayInfoV1;

#define VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionOverlayInfoV1))

typedef struct ViemCompositionOverlayUtf8RangeV1 {
  uint32_t struct_size;
  uint32_t reserved;
  ViemCompositionOverlayIdentityV1 identity;
  uint64_t start;
  uint64_t end;
} ViemCompositionOverlayUtf8RangeV1;

#define VIEM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE \
  ((uint32_t)sizeof(ViemCompositionOverlayUtf8RangeV1))

typedef struct ViemCoreOutcomeV1 {
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
} ViemCoreOutcomeV1;

#define VIEM_CORE_OUTCOME_V1_SIZE ((uint32_t)sizeof(ViemCoreOutcomeV1))

uint32_t viem_core_abi_version(void);

/*
 * All byte sequences are length-delimited; calls do not append a NUL
 * terminator. A null data pointer is permitted only for a zero-length input
 * or capacity query.
 *
 * Core handles own a document, command/controller state, and attached views.
 * They are opaque, process-local, nonzero, and never reused. The core registry
 * lock is never held while invoking a provider callback. Reentrant access to
 * the same checked-out core returns VIEM_STATUS_CORE_BUSY. Destroying a
 * checked-out core does the same without removing it; the caller must retain
 * all provider-owned state and retry destruction after the operation returns.
 */
ViemStatus viem_core_create(const uint8_t *source, uint64_t source_length,
                            const ViemDocumentOptions *options,
                            ViemCoreHandle *out_core,
                            uint64_t *out_revision);

ViemStatus viem_core_destroy(ViemCoreHandle core);
ViemStatus viem_core_revision(ViemCoreHandle core, uint64_t *out_revision);
ViemStatus viem_core_document_state(ViemCoreHandle core,
                                    ViemDocumentStateV1 *out_state);

/*
 * Bounded access to the immutable formatted projection. Snapshot info returns
 * the exact identity used by all dependent operations. The range and mapping
 * copies are all-or-none: a short output returns BUFFER_TOO_SMALL and the
 * complete required byte/element count without writing any output item.
 * Invalid scalar/surrogate boundaries have distinct typed statuses, and an
 * identity mismatch returns STALE_REVISION.
 */
ViemStatus viem_core_formatted_snapshot_info(
    ViemCoreHandle core, ViemFormattedSnapshotInfoV1 *out_info);
ViemStatus viem_core_copy_formatted_utf8_range(
    ViemCoreHandle core, const ViemFormattedUtf8RangeV1 *request,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_map_formatted_utf8_to_utf16(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentityV1 *identity,
    const uint64_t *utf8_offsets, uint64_t offset_count,
    uint64_t *utf16_offsets, uint64_t output_capacity,
    uint64_t *out_required);
ViemStatus viem_core_map_formatted_utf16_to_utf8(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentityV1 *identity,
    const uint64_t *utf16_offsets, uint64_t offset_count,
    uint64_t *utf8_offsets, uint64_t output_capacity,
    uint64_t *out_required);
ViemStatus viem_core_formatted_point_info(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentityV1 *identity,
    uint64_t utf8_offset, ViemFormattedPointInfoV1 *out_info);

/* Snapshot-checked buffer policies; they do not modify source or undo state. */
ViemStatus viem_core_set_read_only(ViemCoreHandle core, uint64_t expected_document, uint64_t expected_revision, uint32_t read_only);
ViemStatus viem_core_mark_recovered(ViemCoreHandle core, uint64_t expected_document, uint64_t expected_revision);

/* Call only after the native write of this exact source snapshot succeeds. */
ViemStatus viem_core_mark_saved(ViemCoreHandle core,
                                const ViemMarkSavedV1 *request);

/*
 * Query/copy one exact immutable normalized style sheet. The copy is all-or-
 * none across every typed array and the UTF-8 string arena. A zero-capacity
 * call returns BUFFER_TOO_SMALL plus exact current sizes. Any identity change
 * returns STALE_REVISION rather than substituting a newer sheet.
 */
/* Identity only: does not resolve or serialize styles. core=0 selects Code. */
ViemStatus viem_core_style_sheet_identity(ViemCoreHandle core,
                                          ViemStyleSheetIdentityV1 *out_identity);
ViemStatus viem_core_style_sheet_info(ViemCoreHandle core,
                                      ViemStyleSheetInfoV1 *out_info);
ViemStatus viem_core_copy_style_sheet(
    ViemCoreHandle core, const ViemStyleSheetIdentityV1 *expected,
    ViemStyleDefinitionV1 *definitions, uint64_t definition_capacity,
    ViemStylePropertyV1 *properties, uint64_t property_capacity,
    ViemStyleValueItemV1 *value_items, uint64_t value_item_capacity,
    ViemStyleDependencyV1 *dependencies, uint64_t dependency_capacity,
    uint8_t *string_bytes, uint64_t string_capacity,
    ViemStyleSheetInfoV1 *out_info);

ViemStatus viem_core_view_add(
    ViemCoreHandle core, const ViemViewOptionsV1 *options,
    const ViemTextMeasurementProviderV1 *provider, ViemViewId *out_view,
    ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_remove(ViemCoreHandle core, ViemViewId view);
/* Bounded speculative layout. Prepare/install are serial coordinator calls.
 * Compute accesses only immutable captured inputs, on a worker with its own
 * compatible AnyWorker provider; callback state must survive computation.
 * Direction is -1 or +1. Prepare returns zero when the nearby band is cached;
 * compute returns zero if cancelled. Each request may be computed once.
 * Install consumes a valid result (including stale rejection); out_installed
 * distinguishes cache installation from discard. It never changes the viewport.
 * Cancel/release may run on any thread. Release requests and uninstalled results
 * exactly once; releasing a request cancels it without waiting for a worker. */
ViemStatus viem_core_view_prepare_prelayout(ViemCoreHandle core, ViemViewId view,
    int32_t direction, uint64_t *out_request);
ViemStatus viem_layout_work_compute(uint64_t request,
    const ViemTextMeasurementProviderV1 *provider, uint64_t *out_result);
ViemStatus viem_core_view_install_prelayout(ViemCoreHandle core, ViemViewId view,
    int32_t direction, uint64_t result, uint8_t *out_installed);
/* Visible table width refinement uses the same immutable worker computation.
 * Prepare returns zero when widths are exact or visible work is already active.
 * Install consumes the result, discards stale dependencies and preserves the
 * anchored viewport. An installed result requires refreshing visible exports.
 * Repeat prepare after successful installation until it returns zero. */
ViemStatus viem_core_view_prepare_table_refinement(ViemCoreHandle core, ViemViewId view,
    uint64_t *out_request);
ViemStatus viem_core_view_install_table_refinement(ViemCoreHandle core, ViemViewId view,
    uint64_t result, uint8_t *out_installed);
ViemStatus viem_layout_work_cancel(uint64_t request);
ViemStatus viem_layout_work_release(uint64_t work);
ViemStatus viem_core_view_state(ViemCoreHandle core, ViemViewId view,
                                ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_capture_restoration(ViemCoreHandle core, ViemViewId view,
    ViemViewRestorationV1 *out_state);
/* Stage positions in the explicitly identified replacement before publication.
 * Input and output must be aligned, readable/writable and mutually disjoint. */
ViemStatus viem_core_view_restore(ViemCoreHandle core, ViemViewId view,
    uint64_t document, uint64_t revision, const ViemViewRestorationV1 *state,
    ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_viewport_state(ViemCoreHandle core, ViemViewId view,
                                         ViemViewportStateV1 *out_state);

/*
 * Snapshot info is an atomic current-layout query. To copy geometry, pass its
 * exact identity to copy_layout_snapshot. Null array pointers are accepted
 * only with zero capacities; BUFFER_TOO_SMALL writes required counts to
 * out_info and writes no array elements. Render-run tokens are borrowed under
 * their provider generation/view lifetime and are not retained by this call.
 */
ViemStatus viem_core_view_layout_snapshot_info(
    ViemCoreHandle core, ViemViewId view, ViemLayoutSnapshotInfoV1 *out_info);
ViemStatus viem_core_view_copy_layout_snapshot(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentityV1 *expected,
    ViemVisualRowV1 *rows, uint64_t row_capacity,
    ViemPositionedClusterV1 *clusters, uint64_t cluster_capacity,
    ViemPositionedCaretV1 *carets, uint64_t caret_capacity,
    ViemLayoutSnapshotInfoV1 *out_info);

/*
 * Paint info is an atomic query against the current immutable layout. Pass its
 * exact identity to copy_layout_paint. Null run output is accepted only with
 * zero capacity; BUFFER_TOO_SMALL writes the required count and fixed paint
 * state to out_info and writes no run elements.
 */
ViemStatus viem_core_view_layout_paint_info(
    ViemCoreHandle core, ViemViewId view, ViemLayoutPaintInfoV1 *out_info);
/* Exact-layout, atomic decorations export; zero-capacity buffers query counts. */
ViemStatus viem_core_view_copy_layout_decorations(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentityV1 *expected,
    ViemLayoutDecorationV1 *decorations, uint64_t decoration_capacity,
    uint8_t *labels, uint64_t label_capacity,
    ViemLayoutDecorationsInfoV1 *out_info);
ViemStatus viem_core_view_copy_layout_paint(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentityV1 *expected,
    ViemPaintStyleRunV1 *runs, uint64_t run_capacity,
    ViemLayoutPaintInfoV1 *out_info);

ViemStatus viem_core_view_caret_geometry(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutCaretRequestV1 *request,
    ViemLayoutCaretGeometryV1 *out_geometry);
ViemStatus viem_core_view_layout_hit_test(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutHitTestRequestV1 *request,
    ViemLayoutCaretPointV1 *out_point);
ViemStatus viem_core_view_presentation(
    ViemCoreHandle core, ViemViewId view,
    ViemViewPresentationV1 *out_presentation);

/*
 * Command-line info is an atomic current-state query. Copy requires its exact
 * identity. Null UTF-8 output is accepted only with zero capacity;
 * BUFFER_TOO_SMALL writes the current required length and no bytes.
 */
ViemStatus viem_core_view_command_line_info(
    ViemCoreHandle core, ViemViewId view, ViemCommandLineInfoV1 *out_info);
ViemStatus viem_core_view_copy_command_line(
    ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentityV1 *expected,
    uint8_t *utf8, uint64_t utf8_capacity,
    ViemCommandLineInfoV1 *out_info);

/*
 * Copy requires an unchanged info record from the atomic current-state query.
 * Any changed session, generation, document revision, or presentation field
 * returns STALE_REVISION. Null output is accepted only with zero capacity.
 * BUFFER_TOO_SMALL writes the required count and no output elements/bytes;
 * other failures clear the count after pointer/record validation. Each call's
 * input, output, and count regions must be aligned and pairwise disjoint.
 */
ViemStatus viem_core_view_completion_info(
    ViemCoreHandle core, ViemViewId view, ViemCompletionInfoV1 *out_info);
ViemStatus viem_core_view_copy_completion_items(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompletionInfoV1 *expected,
    ViemCompletionItemV1 *items, uint64_t capacity, uint64_t *out_count);
ViemStatus viem_core_view_copy_completion_utf8(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompletionInfoV1 *expected,
    uint8_t *bytes, uint64_t capacity, uint64_t *out_count);
/*
 * Advance one bounded search slice without blocking on document-wide work.
 * Poll while SEARCHING is set; changed is 1 when presentation needs refreshing.
 * No active session is a successful no-op. Polling never accepts an item.
 */
ViemStatus viem_core_view_poll_completion(
    ViemCoreHandle core, ViemViewId view, uint8_t *out_changed);
/* Current read-only substitute confirmation prompt. Null/zero count queries
 * use the ordinary UTF-8 two-pass contract; empty means no pending prompt.
 * Reply through the normal key API with y/n/a/q/l or Escape. */
ViemStatus viem_core_view_copy_substitute_confirmation(
    ViemCoreHandle core, ViemViewId view, uint8_t *output,
    uint64_t capacity, uint64_t *out_length);

/*
 * Advance one bounded slice of search highlighting/incremental preview before
 * reading layout and paint. Source and submitted search state are unchanged.
 * changed is 1 when presentation must be exported again. The pending query is
 * read-only; schedule further slices while it returns 1. Both output pointers
 * must identify one writable byte and are cleared on validated-call failures.
 */
ViemStatus viem_core_view_poll_search(
    ViemCoreHandle core, ViemViewId view, uint8_t *out_changed);
ViemStatus viem_core_view_search_work_pending(
    ViemCoreHandle core, ViemViewId view, uint8_t *out_pending);
/*
 * Materialize the selected item before a non-key native command (save, pointer,
 * menu, or IME operation). Refresh presentation before constructing that
 * command's exact target. Inactive completion is a successful no-op; changed
 * is 1 when the document or layout changed. Key input accepts automatically.
 */
ViemStatus viem_core_view_accept_completion(
    ViemCoreHandle core, ViemViewId view, uint8_t *out_changed);

/*
 * Visual-selection info resolves the current selection against the exact
 * current layout. Copy requires both its layout and opaque state identities.
 * Null array pointers are accepted only with zero capacities;
 * BUFFER_TOO_SMALL writes both required counts and no array elements.
 * STALE_REVISION means either identity changed. Character/Line selections
 * retain their complete logical segments when endpoints leave layout coverage;
 * rectangle_count may be zero when selected text is entirely offscreen.
 * OUTSIDE_LAYOUT_COVERAGE may still mean a Visual Block's row-dependent extent
 * cannot be resolved against the current materialized layout.
 */
ViemStatus viem_core_view_visual_selection_info(
    ViemCoreHandle core, ViemViewId view,
    ViemVisualSelectionInfoV1 *out_info);
ViemStatus viem_core_view_copy_visual_selection(
    ViemCoreHandle core, ViemViewId view,
    const ViemVisualSelectionIdentityV1 *expected,
    ViemVisualSelectionSegmentV1 *segments, uint64_t segment_capacity,
    ViemVisualSelectionRectangleV1 *rectangles, uint64_t rectangle_capacity,
    ViemVisualSelectionInfoV1 *out_info);

/*
 * Query source-backed Strong/Emphasis state and capability using only the
 * current core-owned logical selection. No layout snapshot is required.
 */
ViemStatus viem_core_view_semantic_style_presentation(
    ViemCoreHandle core, ViemViewId view, uint32_t style,
    ViemSemanticStylePresentationV1 *out_presentation);

/* Apply to the exact logical-selection identity returned by the query. */
ViemStatus viem_core_view_set_semantic_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetSemanticStyleV1 *request,
    ViemCoreOutcomeV1 *out_outcome);

/*
 * Bind an exact current Visual selection as the literal forward-search target.
 * The selection identity includes both layout and opaque controller state;
 * stale requests fail rather than capturing different text.
 */
ViemStatus viem_core_view_use_selection_for_find(
    ViemCoreHandle core, ViemViewId view,
    const ViemVisualSelectionIdentityV1 *expected,
    ViemCoreOutcomeV1 *out_outcome);

/* Reveal the active endpoint of the core-owned Visual selection. */
ViemStatus viem_core_view_reveal_selection(
    ViemCoreHandle core, ViemViewId view,
    ViemCoreOutcomeV1 *out_outcome);

/*
 * Host-context turns consume immutable per-turn clipboard snapshots and
 * writable capabilities. On success, out_effect_batch is zero or an owned
 * handle which must be released. The effect batch survives core destruction.
 */
ViemStatus viem_core_view_send_key_with_host_context_v2(
    ViemCoreHandle handle, ViemViewId view, const ViemKeyInputV1 *input,
    const ViemCommandTurnContextV2 *context, ViemCoreOutcomeV1 *out_outcome,
    ViemEffectBatchHandle *out_effect_batch);
ViemStatus viem_core_view_send_text_with_host_context_v2(
    ViemCoreHandle handle, ViemViewId view, const uint8_t *text, uint64_t text_length,
    const ViemCommandTurnContextV2 *context, ViemCoreOutcomeV1 *out_outcome,
    ViemEffectBatchHandle *out_effect_batch);

/* Pending multi-key mapping state and timeout dispatch. Flush captures host
 * context and returns an owned effect batch just like send_key. */
ViemStatus viem_core_view_has_pending_mapping(ViemCoreHandle core, ViemViewId view,
                                             uint8_t *pending);
ViemStatus viem_core_view_flush_mapping_with_host_context_v2(
    ViemCoreHandle core, ViemViewId view, const ViemCommandTurnContextV2 *context,
    ViemCoreOutcomeV1 *outcome, ViemEffectBatchHandle *effects);

ViemStatus viem_effect_batch_info(ViemEffectBatchHandle batch,
                                  ViemEffectBatchInfoV1 *out_info);
/*
 * Copy is all-or-none across every typed array and the UTF-8 arena. A short
 * capacity returns BUFFER_TOO_SMALL plus complete required sizes and writes no
 * array/arena element. These are raw Ex requests: the batch performs no I/O,
 * does not own a prepared artifact-write lifecycle, and does not establish a
 * save point. Route native work and acknowledge exact successful saves through
 * viem_core_mark_saved separately.
 */
ViemStatus viem_effect_batch_copy(
    ViemEffectBatchHandle batch,
    ViemClipboardWriteV1 *clipboard_writes,
    uint64_t clipboard_write_capacity,
    ViemExFrontendRequestV1 *ex_requests,
    uint64_t ex_request_capacity,
    ViemExOptionDisplayV1 *ex_options,
    uint64_t ex_option_capacity,
    ViemExMarkV1 *ex_marks,
    uint64_t ex_mark_capacity,
    ViemExRegisterV1 *ex_registers,
    uint64_t ex_register_capacity,
    ViemExJumpV1 *ex_jumps,
    uint64_t ex_jump_capacity,
    ViemExTextLineV1 *ex_text_lines,
    uint64_t ex_text_line_capacity,
    uint32_t *file_formats,
    uint64_t file_format_capacity,
    uint64_t *hard_breaks,
    uint64_t hard_break_capacity,
    uint8_t *string_bytes,
    uint64_t string_capacity,
    ViemEffectBatchInfoV1 *out_info);
ViemStatus viem_effect_batch_release(ViemEffectBatchHandle batch);

ViemStatus viem_core_view_place_cursor(
    ViemCoreHandle core, ViemViewId view,
    const ViemPlaceCursorV1 *request, ViemCoreOutcomeV1 *out_outcome);

/* Select all logical content through EOF, independently of line policy. */
ViemStatus viem_core_view_select_all(
    ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision,
    ViemCoreOutcomeV1 *out_outcome);

/* Selection optionKind is VIEM_EX_OPTION_KEYMODEL/SELECTMODE/AUTOSELECT.
 * AUTOSELECT uses the UTF-8 boolean spelling "1" or "0".
 * Copy reports required UTF8 bytes, with no terminator. Setter updates all
 * views without dispatching input or disturbing their mode/pending command. */
ViemStatus viem_core_copy_selection_option(ViemCoreHandle core, uint32_t option_kind,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_set_selection_option(ViemCoreHandle core, uint32_t option_kind,
    const uint8_t *value, uint64_t value_length);

#define VIEM_SELECTION_ORIGIN_MOUSE 1u
#define VIEM_SELECTION_ORIGIN_KEY 2u
#define VIEM_SELECTION_ORIGIN_COMMAND 3u
/* Choose native Selection or Vim Select/Visual using the configured options.
 * return_mode is NORMAL, INSERT, or REPLACE: native navigation resumes this
 * mode when it ends the selection. It does not alter the selected extent. */
ViemStatus viem_core_view_set_selection_origin(
    ViemCoreHandle core, ViemViewId view, uint32_t origin, uint32_t return_mode,
    ViemCoreOutcomeV1 *out_outcome);

/* Enter Normal mode and reveal the first nonblank grapheme on a logical hard
 * line in the exact document revision. Lines are one-based; zero selects the
 * first and excess (including UINT64_MAX) selects the last. Pending input is
 * cancelled without executing it, changing source, or recording a macro key. */
ViemStatus viem_core_view_go_to_line(
    ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision, uint64_t line,
    ViemCoreOutcomeV1 *out_outcome);

/* Native Edit-menu history navigation; behavior is independent of Vim mode. */
ViemStatus viem_core_view_undo(ViemCoreHandle core, ViemViewId view,
                               ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_redo(ViemCoreHandle core, ViemViewId view,
                               ViemCoreOutcomeV1 *out_outcome);

/*
 * Composition requests are exact-revision operations. Begin replacement and
 * update selection ranges use UTF-8 byte offsets and must also be extended-
 * grapheme boundaries. Update and cancel never edit source. Commit applies its
 * final UTF-8 payload as one source transaction and one undo unit. On a model
 * policy failure, source stays exact and the final payload remains marked so
 * the frontend can cancel or retry under another policy.
 */
ViemStatus viem_core_view_composition_begin(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionBeginV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_composition_update(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionUpdateV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_composition_overlay_info(
    ViemCoreHandle core, ViemViewId view,
    ViemCompositionOverlayInfoV1 *out_info);
ViemStatus viem_core_view_copy_composition_utf8_range(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionOverlayUtf8RangeV1 *request, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_composition_commit(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionCommitV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_composition_cancel(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionCancelV1 *request, ViemCoreOutcomeV1 *out_outcome);

/*
 * Horizontal-only origin changes do not shape, reflow, or replace immutable
 * layout. HAS_TOP validates the supplied identity and atomically installs one
 * bounded regional viewport; a stale identity changes no view state.
 */
ViemStatus viem_core_view_set_viewport_origin(
    ViemCoreHandle core, ViemViewId view,
    const ViemViewportOriginV1 *request, ViemCoreOutcomeV1 *out_outcome);

/* Theme padding is presentation-only and scrolls with the document canvas. */
ViemStatus viem_core_view_set_padding(ViemCoreHandle core, ViemViewId view,
    float top, float left, float bottom, float right);
ViemStatus viem_core_view_resize(ViemCoreHandle core, ViemViewId view,
                                 float width, float height,
                                 ViemCoreOutcomeV1 *out_outcome);
/*
 * Change only this view's magnification. The scale must be finite and within
 * 0.25 through 5.0 inclusive. Shaping/wrapping/layout are refreshed synchronously; source and
 * semantic projections are unchanged.
 */
/* Adjacent zoom stop (25%..500% inclusive); increasing must be 0 or 1.
 * Saturates at endpoints; invalid input returns INVALID_ARGUMENT and zero. */
ViemStatus viem_core_adjacent_zoom_scale(float scale, uint32_t increasing,
                                       float *out_scale);
/* Arbitrary finite view scales within 0.25..5.0 are accepted. */
ViemStatus viem_core_view_set_scale(ViemCoreHandle core, ViemViewId view,
                                    float scale,
                                    ViemCoreOutcomeV1 *out_outcome);
#define VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT 1u
#define VIEM_LINE_LOCATION_FRAGMENT_EXACT 2u
typedef struct ViemViewLineLocationV1 {
    uint32_t struct_size;
    uint32_t mode;
    uint32_t flags;
    uint32_t reserved;
    uint64_t line;
    uint64_t column;
    uint64_t hard_line;
    uint64_t fragment;
} ViemViewLineLocationV1;
#define VIEM_VIEW_LINE_LOCATION_V1_SIZE ((uint32_t)sizeof(ViemViewLineLocationV1))
/* One-based fields; line==0 means use exact hard_line/fragment fallback. */
ViemStatus viem_core_view_line_location(ViemCoreHandle handle, ViemViewId view, ViemViewLineLocationV1 *out_location);

#define VIEM_LINE_MODE_VISUAL 0u
#define VIEM_LINE_MODE_PHYSICAL_SOURCE 1u
/* View-local visual or physical source-line command policy. */
/* Markdown flows structurally; Markdown Source defaults off per view.
   Setter is supported for Markdown Source only. */
ViemStatus viem_core_view_paragraph_flow(ViemCoreHandle core, ViemViewId view, uint32_t *out_enabled);
ViemStatus viem_core_view_set_paragraph_flow(ViemCoreHandle core, ViemViewId view, uint32_t enabled, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_line_mode(ViemCoreHandle core, ViemViewId view, uint32_t *out_mode);
/* Base Paragraph font size with line spacing and this view's zoom applied.
 * Does not perform layout or consult the visible text's styles/font metrics. */
ViemStatus viem_core_view_default_line_height(ViemCoreHandle core, ViemViewId view, float *out_height);
ViemStatus viem_core_view_default_column_width(ViemCoreHandle handle, ViemViewId view, float *out_width);
ViemStatus viem_core_view_set_line_mode(ViemCoreHandle core, ViemViewId view, uint32_t mode, ViemCoreOutcomeV1 *out_outcome);
/* Application input preference; enabled must be 0 or 1. No source/undo change. */
ViemStatus viem_core_view_set_smart_quotes(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
ViemStatus viem_core_view_set_markdown_autodetect(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
ViemStatus viem_core_view_set_wrap(ViemCoreHandle core, ViemViewId view,
                                   uint32_t wrap,
                                   ViemCoreOutcomeV1 *out_outcome);
/* Deprecated compatibility entry point: 1 is a no-op; 0 is invalid. */
ViemStatus viem_core_view_set_linebreak(ViemCoreHandle core, ViemViewId view,
                                        uint32_t linebreak,
                                        ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_set_file_format(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetFileFormatV1 *request, ViemCoreOutcomeV1 *out_outcome);

ViemStatus viem_core_view_list_selection(
    ViemCoreHandle core, ViemViewId view,
    ViemLogicalSelectionIdentityV1 *out_selection);
ViemStatus viem_core_view_table_context(ViemCoreHandle core, ViemViewId view,
    ViemTableContextV1 *out_context);
ViemStatus viem_core_view_table_context_at(ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision, uint64_t text_offset,
    ViemTableContextV1 *out_context);
ViemStatus viem_core_view_insert_table(ViemCoreHandle core, ViemViewId view,
    const ViemInsertTableV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_table_action(ViemCoreHandle core, ViemViewId view,
    const ViemTableActionV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_copy_table_cells(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentityV1 *expected, ViemTableCellV1 *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_table_selection(ViemCoreHandle core, ViemViewId view,
    ViemTableSelectionV1 *out_selection);
ViemStatus viem_core_view_select_table_cells(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelectionV1 *request, ViemCoreOutcomeV1 *out_outcome);
/* Complete exact cell selection: no clipboard effects; row-major ranges include
 * offscreen/empty cells. Pointer regions must be pairwise disjoint. */
ViemStatus viem_core_view_copy_table_selection_text(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelectionV1 *expected, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_copy_table_selection_ranges(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelectionV1 *expected, ViemFormattedUtf8RangeV1 *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_set_block_quote(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetBlockQuoteV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_set_list_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetListStyleV1 *request, ViemCoreOutcomeV1 *out_outcome);
/* Exact current selection; capability bits reflect verified structural edits. */
ViemStatus viem_core_view_list_indent_capabilities(
    ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentityV1 *expected_selection, uint32_t *out_flags);
/* unindent is 0 or 1. A top-level item cannot be unindented. */
ViemStatus viem_core_view_indent_list(
    ViemCoreHandle core, ViemViewId view,
    const ViemListIndentV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_set_paragraph_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetParagraphStyleV1 *request, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_assign_style(
    ViemCoreHandle handle, ViemViewId view,
    const ViemAssignStyleV1 *request, ViemCoreOutcomeV1 *out_outcome);
/*
 * Begin one exact frontend-owned style gesture. Only one may be active in a
 * core. An empty group creates no history entry. Any ordinary coordinator
 * event or removal of the owner closes the successful prefix and consumes the
 * capability before continuing.
 */
ViemStatus viem_core_view_begin_style_edit_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleSheetIdentityV1 *expected,
    ViemStyleEditGroupV1 *out_group);
/* One exact editable-definition field edit through its source/configuration
 * authority, with a standalone undo unit. */
ViemStatus viem_core_view_edit_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEditV1 *request, ViemCoreOutcomeV1 *out_outcome);
/*
 * Commit one immediately visible exact edit into group. request must carry the
 * current style-sheet identity on every call. A failed/no-op edit leaves the
 * group usable and does not poison previously successful edits.
 */
ViemStatus viem_core_view_edit_style_in_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEditGroupV1 *group,
    const ViemStyleEditV1 *request, ViemCoreOutcomeV1 *out_outcome);
/*
 * Consume group and close its successful edits as one undo/redo unit. This
 * never rolls back. Reuse after end returns INVALID_STYLE_EDIT_GROUP.
 */
ViemStatus viem_core_view_end_style_edit_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEditGroupV1 *group);

/* Set Markdown strikethrough at an exact selection or typing caret. */
ViemStatus viem_core_view_set_strikethrough(ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentityV1 *expected_selection, uint8_t enabled,
    ViemCoreOutcomeV1 *out_outcome);
/* Unscaled caret en width for the exact document revision, including empty lines. */
ViemStatus viem_core_view_font_en_width(ViemCoreHandle core, ViemViewId view,
    uint64_t expected_revision, float *out_width);

/* Returns the semantic Off/On/Mixed constants for strikethrough. */
ViemStatus viem_core_view_strikethrough_state(ViemCoreHandle core, ViemViewId view,
    uint32_t *out_state);

ViemStatus viem_core_copy_source_bytes(ViemCoreHandle core,
                                       uint64_t expected_revision,
                                       uint8_t *output,
                                       uint64_t output_capacity,
                                       uint64_t *out_required);
/* Capture a standalone UTF-8 HTML export without changing the editing session.
 * Call with the frontend's other serialized core operations. No providers run
 * during capture. The handle owns an immutable snapshot that survives editing,
 * view closure, and core destruction. On failure out_export is zero (provided
 * its storage is valid). Release the handle after rendering/copying or failure. */
ViemStatus viem_core_prepare_html_export(ViemCoreHandle core,
                                         ViemViewId view,
                                         uint64_t expected_revision,
                                         ViemHtmlExportHandle *out_export);
/* Render a prepared handle once, then copy its bytes. This may wait for syntax
 * workers and should run on a background thread. It never accesses the live core
 * or invokes its measurement provider. Markdown Source exports semantics; Code
 * analyzes the complete captured snapshot, including offscreen text.
 * Rendering twice, or copying before render succeeds, returns INVALID_ARGUMENT. */
ViemStatus viem_html_export_render(ViemHtmlExportHandle export_handle);
/* Two-pass copy from one owned result. BUFFER_TOO_SMALL reports the exact size
 * without writing output. Output and out_required must be disjoint. */
ViemStatus viem_html_export_copy_utf8(ViemHtmlExportHandle export_handle,
                                     uint8_t *output,
                                     uint64_t output_capacity,
                                     uint64_t *out_required);
ViemStatus viem_html_export_release(ViemHtmlExportHandle export_handle);
ViemStatus viem_core_copy_formatted_utf8(ViemCoreHandle core,
                                         uint64_t expected_revision,
                                         uint8_t *output,
                                         uint64_t output_capacity,
                                         uint64_t *out_required);

/* Selected named style identities. Code includes its displayed automatic
 * character style; other formats exclude automatic syntax decoration.
 * UTF-8 IDs are concatenated paragraph first, then character. Mixed roles
 * have zero bytes. The output is tied to the exact document revision. */
#define VIEM_SELECTED_STYLE_PARAGRAPH_MIXED (1u << 0)
#define VIEM_SELECTED_STYLE_CHARACTER_MIXED (1u << 1)
/* Structural membership, independent of style assignment and list depth. */
#define VIEM_SELECTED_STYLE_HAS_BULLETS (1u << 2)
#define VIEM_SELECTED_STYLE_HAS_NUMBERING (1u << 3)
#define VIEM_SELECTED_STYLE_HAS_NON_LIST (1u << 4)
#define VIEM_SELECTED_STYLE_HAS_QUOTES (1u << 5)
#define VIEM_SELECTED_STYLE_HAS_NON_QUOTE (1u << 6)
#define VIEM_SELECTED_STYLE_HAS_TABLE (1u << 7)
#define VIEM_SELECTED_STYLE_HAS_CODE_BLOCK (1u << 8)
typedef struct ViemSelectedStylesInfoV1 {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
  uint64_t paragraph_id_bytes;
  uint64_t character_id_bytes;
} ViemSelectedStylesInfoV1;
ViemStatus viem_core_view_selected_styles_export(ViemCoreHandle core,
    ViemViewId view, uint64_t expected_revision,
    ViemSelectedStylesInfoV1 *out_info, uint8_t *out_utf8, uint64_t capacity);

/* Version 1 UTF-8 JSON defaults. Initialization requires a pristine core with no views;
 * it does not change source, revision, dirty state, or history. Export is two-pass,
 * reports required bytes, and never writes a partial output. Invalid entries or
 * declarations are ignored with diagnostics while valid defaults are applied.
 * An unreadable or unsupported file leaves all existing defaults unchanged.
 * Diagnostic callbacks run synchronously after releasing the core lease;
 * message bytes are borrowed only for the duration of each callback. */
typedef void (*ViemStyleDefaultsDiagnosticCallback)(void *context, const uint8_t *message, uint64_t length);
ViemStatus viem_core_initialize_style_defaults(ViemCoreHandle core, uint64_t expected_revision, const uint8_t *json, uint64_t length, ViemStyleDefaultsDiagnosticCallback diagnostic, void *context);
/* Live replacement uses the same v1 sheet schema and diagnostics. It preserves
 * source, revision, dirty/savepoint state and undo history, refreshes all views,
 * and keeps source-authored declarations authoritative. Code uses its global
 * stylesheet API below. The current application defaults survive undo/redo. */
ViemStatus viem_core_replace_style_defaults(ViemCoreHandle core, uint64_t expected_revision, const uint8_t *json, uint64_t length, ViemStyleDefaultsDiagnosticCallback diagnostic, void *context);
ViemStatus viem_core_export_style_defaults(ViemCoreHandle core, uint64_t expected_revision, uint8_t *output, uint64_t capacity, uint64_t *required);

/* Portable aggregate theme v1: appearance plus optional text/markdown v1
 * and code v3 style sheets. Validation is atomic and never installs styles.
 * Omitted sheets use built-in defaults. Input is bounded to 20 MiB. Default
 * export emits every sheet, uses the ordinary two-pass/disjoint output contract,
 * and requires no files. Name validation accepts a UTF-8 filename stem of at
 * most 32 characters, excluding reserved Default and Windows device names. */
#define VIEM_THEME_PRESET_MIDNIGHT 0u
#define VIEM_THEME_PRESET_PAPER 1u
ViemStatus viem_theme_default_json(uint32_t preset, uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_theme_validate_json(const uint8_t *json, uint64_t length);
ViemStatus viem_theme_validate_name(const uint8_t *name, uint64_t length);

/* Command prompt selection and editing carry the exact exported prompt identity. */
typedef struct ViemCommandLineSelectionV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t anchor_utf8_offset;
  uint64_t active_utf8_offset;
} ViemCommandLineSelectionV1;
ViemStatus viem_core_view_command_line_selection(ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentityV1 *expected, ViemCommandLineSelectionV1 *out_selection);
ViemStatus viem_core_view_edit_command_line(ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentityV1 *expected, uint32_t operation,
    uint64_t start, uint64_t end, const uint8_t *utf8, uint64_t length,
    ViemCoreOutcomeV1 *out_outcome);

ViemStatus viem_core_view_set_markdown_source_with_effects(ViemCoreHandle core, ViemViewId view, const ViemSetMarkdownSourceV1 *request, ViemCoreOutcomeV1 *out_outcome, ViemEffectBatchHandle *out_effects);
ViemStatus viem_core_view_set_encoding_with_effects(ViemCoreHandle core, ViemViewId view, const ViemSetEncodingV1 *request, ViemCoreOutcomeV1 *out_outcome, ViemEffectBatchHandle *out_effects);
ViemStatus viem_core_copy_hard_line_source_bytes(ViemCoreHandle core, uint64_t document, uint64_t revision, uint64_t first_line, uint64_t end_line, uint8_t *output, uint64_t capacity, uint64_t *out_required, uint32_t *out_complete);

/* Versioned clipboard JSON exports use the ordinary two-call buffer contract.
 * Ranges are exact formatted snapshots. Effects remain valid after mutations.
 * source_text is the decoded authored source fragment; source_bytes retains its
 * encoding. Only is_rich payloads should be published as private/rich types. */
/* Passive rich clipboard import, independent of document source formats. */
#define VIEM_CLIPBOARD_FORMAT_HTML 1u
ViemStatus viem_import_clipboard_json(uint32_t format, const uint8_t *source,
    uint64_t source_length, uint8_t *output, uint64_t output_capacity,
    uint64_t *out_required);

ViemStatus viem_core_copy_clipboard_json(
    ViemCoreHandle handle, const ViemFormattedUtf8RangeV1 *request,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_effect_batch_copy_clipboard_json(
    ViemEffectBatchHandle batch, uint64_t clipboard_index,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);

/* Application-wide Code stylesheet. Its identity has document/revision zero.
 * Changes never create a buffer transaction. UTF-8/typed output arrays use
 * the same exact-capacity and disjoint-pointer rules as document styles. */
ViemStatus viem_code_style_sheet_info(ViemStyleSheetInfoV1 *output);
ViemStatus viem_code_copy_style_sheet(const ViemStyleSheetIdentityV1 *expected,
    ViemStyleDefinitionV1 *definitions, uint64_t definition_capacity,
    ViemStylePropertyV1 *properties, uint64_t property_capacity,
    ViemStyleValueItemV1 *value_items, uint64_t value_item_capacity,
    ViemStyleDependencyV1 *dependencies, uint64_t dependency_capacity,
    uint8_t *string_bytes, uint64_t string_capacity, ViemStyleSheetInfoV1 *out_info);
ViemStatus viem_code_edit_style(const ViemStyleEditV1 *request, ViemStyleSheetInfoV1 *output);
ViemStatus viem_code_create_style(const ViemCreateStyleV1 *request, ViemStyleSheetInfoV1 *output);
ViemStatus viem_code_delete_style(const ViemDeleteStyleV1 *request, ViemStyleSheetInfoV1 *output);
/* Generates the implicit definition for one syntax name, and its missing
 * dotted ancestors, when it has no definition. Fails with ResourceExhausted
 * when the implicit-definition limit leaves the name undefined. */
ViemStatus viem_code_materialize_style(const uint8_t *name, uint64_t length,
    ViemStyleSheetInfoV1 *output);
ViemStatus viem_code_replace_style_json(const uint8_t *input, uint64_t length);
ViemStatus viem_code_export_style_json(uint8_t *output, uint64_t capacity, uint64_t *required);
/* Buffer-local source-preserving mode override. Auto returns to detection. */
#define VIEM_DOCUMENT_MODE_AUTO 0u
#define VIEM_DOCUMENT_MODE_PLAIN_TEXT 1u
#define VIEM_DOCUMENT_MODE_MARKDOWN 2u
#define VIEM_DOCUMENT_MODE_CODE 3u
typedef struct ViemSetDocumentModeV1 {
    uint32_t struct_size;
    uint32_t mode;
    uint64_t document_id;
    uint64_t document_revision;
    uint32_t formatted_markdown;
    uint32_t reserved;
} ViemSetDocumentModeV1;
#define VIEM_SET_DOCUMENT_MODE_V1_SIZE ((uint32_t)sizeof(ViemSetDocumentModeV1))
/* Only Code accepts a nonempty language ID. All input/output regions disjoint. */
ViemStatus viem_core_view_set_document_mode_with_effects(ViemCoreHandle core, ViemViewId view,
    const ViemSetDocumentModeV1 *request, const uint8_t *language, uint64_t length,
    ViemCoreOutcomeV1 *out_outcome, ViemEffectBatchHandle *out_effects);
/* Two-pass UTF-8 JSON queries. Output regions must be disjoint. Mode reads
   cached detection; language enumeration does not load or execute providers. */
ViemStatus viem_core_copy_document_mode_json(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_copy_code_languages_json(uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_core_initialize_code_detection(ViemCoreHandle core, const uint8_t *filename, uint64_t length, uint8_t allow_auto_code);
ViemStatus viem_core_configure_syntax(ViemCoreHandle core, const uint8_t *vim_directory, uint64_t length);
ViemStatus viem_core_set_code_filename_associations_json(ViemCoreHandle core, const uint8_t *json, uint64_t length);
/* selection: Automatic=0, None=1, Language=2. Only Language takes a nonempty name. */
ViemStatus viem_core_set_code_language(ViemCoreHandle core, uint32_t selection, const uint8_t *language, uint64_t length);
ViemStatus viem_core_redetect_code_language(ViemCoreHandle core, const uint8_t *filename, uint64_t length);
/* Link popup JSON: selection {viewId,documentId,revision,start,end,kind,anchor,
 * active,affinity}, canInsert, text, link null or {start,end,text,destination,
 * editable}. Query work is bounded to the active source region. */
ViemStatus viem_core_view_copy_link_context(ViemCoreHandle core, ViemViewId view,
                                           uint8_t *output, uint64_t capacity,
                                           uint64_t *required);
/* action: 0 inserts at expected selection, 1 edits, 2 removes active link.
 * Every mutation validates the complete expected logical selection identity. */
ViemStatus viem_core_view_edit_link(ViemCoreHandle core, ViemViewId view,
                                   const ViemLogicalSelectionIdentityV1 *expected,
                                   uint32_t action, uint64_t link_start,
                                   uint64_t link_end, ViemUtf8Slice text,
                                   ViemUtf8Slice destination,
                                   ViemCoreOutcomeV1 *outcome);
/* Image popup has the link-context schema with "image" instead of "link".
 * WYSIWYG range is one atomic U+FFFC object, Source range is full notation.
 * No query fetches resources. Empty alternative text is allowed. */
ViemStatus viem_core_view_copy_image_context(ViemCoreHandle core, ViemViewId view,
                                            uint8_t *output, uint64_t capacity,
                                            uint64_t *required);
/* action: 0 inserts, 1 edits, 2 removes the whole image. */
ViemStatus viem_core_view_edit_image(ViemCoreHandle core, ViemViewId view,
                                    const ViemLogicalSelectionIdentityV1 *expected,
                                    uint32_t action, uint64_t image_start,
                                    uint64_t image_end, ViemUtf8Slice text,
                                    ViemUtf8Slice destination,
                                    ViemCoreOutcomeV1 *outcome);
/* Select the complete WYSIWYG image independently of pointer preferences. */
ViemStatus viem_core_view_select_image(ViemCoreHandle core, ViemViewId view,
                                      uint64_t document_id, uint64_t revision,
                                      uint64_t text_offset, ViemCoreOutcomeV1 *outcome);
/* fragment is percent-decoded, excludes '#'; outputs name this exact revision. */
ViemStatus viem_core_find_link_fragment(ViemCoreHandle core, uint64_t document_id,
                                       uint64_t revision, ViemUtf8Slice fragment,
                                       uint64_t *offset, uint8_t *found);

/* Exact snapshot, two-pass UTF-8. Outputs must be disjoint. found distinguishes
 * no link from a link with an empty destination. No source or view mutation. */
ViemStatus viem_core_copy_link_destination(ViemCoreHandle core,
    uint64_t document_id, uint64_t revision, uint64_t text_offset,
    uint8_t *output, uint64_t capacity, uint64_t *required, uint8_t *found);
/* Application default for textwidth (positive). Buffer :set overrides survive. */
ViemStatus viem_core_set_text_width_default(ViemCoreHandle core, uint32_t width);
/* Application defaults; JSON is validated before mutation. Ex overrides survive. */
ViemStatus viem_validate_whitespace_presentation(const uint8_t *json, uint64_t length);
ViemStatus viem_core_set_indentation_defaults(ViemCoreHandle core, const uint8_t *json, uint64_t length);
ViemStatus viem_core_set_whitespace_presentation_defaults(ViemCoreHandle core, const uint8_t *json, uint64_t length);
ViemStatus viem_core_view_set_visible_whitespace(ViemCoreHandle core, ViemViewId view, uint8_t enabled);
/* Exact presentation-snapshot and viewport UTF-8 JSON batch:
   {style, markers:[{text,rowIndex,x,y,width,height}], enabled, applicable}.
   All input/output regions must be disjoint. Count and copy pass the same
   finite viewport (nonnegative origin and dimensions). A changed viewport
   returns VIEM_STATUS_STALE_REVISION even when cached layout is reused. */
ViemStatus viem_core_view_copy_whitespace_markers(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentityV1 *expected, const ViemLayoutRectV1 *expected_viewport,
    uint8_t *output, uint64_t capacity, uint64_t *out_length);
ViemStatus viem_core_poll_syntax(ViemCoreHandle core, uint8_t *changed);
/* Wait up to the shared 100 ms syntax grace period, without holding the core.
 * Materialize the viewport first; refresh invalidated layout when changed.
 * Repeated presentation of the same timed-out request does not wait again. */
ViemStatus viem_core_view_wait_for_syntax(ViemCoreHandle core, ViemViewId view, uint8_t *changed);
ViemStatus viem_core_copy_syntax_diagnostics(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);
/* Read up to 8 KiB of distinct warnings for this exact immutable layout.
 * Standard two-pass UTF-8 output; stale snapshots are rejected, never replaced. */
ViemStatus viem_core_view_copy_layout_diagnostics(ViemCoreHandle core, ViemViewId view, const ViemLayoutSnapshotIdentityV1 *expected, uint8_t *output, uint64_t capacity, uint64_t *required);
/* Read-only two-pass UTF-8 JSON array of unique sorted names in accepted syntax
   runs, including undefined names. Does not parse, publish, or scan source. */
ViemStatus viem_core_copy_syntax_style_names(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);

/* Host-loaded Ex continuations. Source files are UTF-8, at most 1 MiB,
   with at most 10,000 executed lines per invocation and 16 nested files. */
#define VIEM_SOURCE_MAX_BYTES 1048576u
#define VIEM_SOURCE_MAX_COMMANDS 10000u
#define VIEM_SOURCE_MAX_DEPTH 16u
ViemStatus viem_core_view_read_file(ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t revision, uint64_t after_line,
    const uint8_t *bytes, uint64_t length, ViemCoreOutcomeV1 *out_outcome);
ViemStatus viem_core_view_source_line(ViemCoreHandle core, ViemViewId view,
    uint32_t depth, const uint8_t *text, uint64_t length,
    const ViemCommandTurnContextV2 *context, ViemCoreOutcomeV1 *out_outcome,
    ViemEffectBatchHandle *out_effects);

#ifdef __cplusplus
}
#endif

#endif /* VIEM_CORE_H */
