#ifndef VIEM_CORE_H
#define VIEM_CORE_H

#include <stdint.h>
#include "viem_startup.h"

#ifdef __cplusplus
extern "C" {
#endif

#define VIEM_CORE_ABI_VERSION 9u
#define VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION 4u

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
#define VIEM_STATUS_UNFINISHED_COMMENT 41u
#define VIEM_STATUS_INTERNAL_ERROR 254u
#define VIEM_STATUS_PANIC 255u

/* Frontend-owned window geometry; root pane ID is 1. IDs/handles never reuse.
 * Orientations: 0 stacked, 1 side by side. At most 256 panes (511 frames).
 * Failed mutations leave geometry unchanged. All calls are serial per handle.
 * Copy outputs must be aligned and disjoint; policy-required means no room. */
typedef struct ViemPaneChrome { uint64_t id; double status_height; } ViemPaneChrome;
#define VIEM_PANE_FRAME_STATUS_DRAGGABLE 1u
/* Interior vertical splitter top edge fully meets horizontal status-bar chrome.
 * Outer-top, aligned cross and partially covered junctions remain unflagged. */
#define VIEM_PANE_FRAME_SUPPRESS_SPLITTER_TOP_BORDER (1u << 1)
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
ViemStatus viem_pane_layout_split(uint64_t handle, uint64_t pane, uint32_t orientation, double status_height, uint32_t placement, uint64_t *out_pane);
ViemStatus viem_pane_layout_remove(uint64_t handle, uint64_t pane);
/* kind 0 drags a pane's status bar, 1 a splitter. A status drag preview-closes
 * the panes below its boundary, bottom first, when the bar would overlap them
 * and every one is listed in closable (none for splitters). Moving back to
 * where a pane closed restores it exactly. Frames omit closed panes until
 * end_drag or any structural change commits them. */
ViemStatus viem_pane_layout_drag(uint64_t handle, uint64_t id, uint32_t kind, double delta,
    const uint64_t *closable, uint64_t closable_count);
ViemStatus viem_pane_layout_end_drag(uint64_t handle);
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
/* Markdown Preview, or literal Code whose effective language is Markdown. */
#define VIEM_DOCUMENT_STATE_MARKDOWN_SOURCE (1u << 6)

typedef struct ViemDocumentOptions {
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
} ViemDocumentOptions;


/* Model, pipeline, and history metadata captured in one serial core query. */
typedef struct ViemDocumentState {
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
} ViemDocumentState;


/* Exact identity of one immutable formatted projection. */
typedef struct ViemFormattedSnapshotIdentity {
  uint64_t document_id;
  uint64_t document_revision;
} ViemFormattedSnapshotIdentity;


/* Constant-time aggregate metadata; querying it never flattens document text. */
typedef struct ViemFormattedSnapshotInfo {
  ViemFormattedSnapshotIdentity identity;
  uint64_t utf8_length;
  uint64_t utf16_length;
  uint64_t hard_line_count;
} ViemFormattedSnapshotInfo;


/* Scalar-aligned, half-open UTF-8 range in one exact formatted snapshot. */
typedef struct ViemFormattedUtf8Range {
  ViemFormattedSnapshotIdentity identity;
  uint64_t utf8_start;
  uint64_t utf8_end;
} ViemFormattedUtf8Range;


/*
 * Logical status metadata for one grapheme-aligned formatted point. Line and
 * column values are zero based. hard_line_start..hard_line_end excludes the
 * following semantic hard break.
 */
typedef struct ViemFormattedPointInfo {
  ViemFormattedSnapshotIdentity identity;
  uint64_t utf8_offset;
  uint64_t utf16_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
} ViemFormattedPointInfo;


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
/* Distinct keypad identity: 20 Home, 21 End, 22 PageUp, 23 PageDown,
 * 24 Insert, 25 Delete, 26 Plus, 27 Minus, 28 Multiply, 29 Divide,
 * 30 Enter, 31 Point, 40..49 digits 0..9. Modifiers use the normal bits. */
#define VIEM_KEY_KEYPAD 34
#define VIEM_KEY_FUNCTION 22u
#define VIEM_KEY_COPY_SELECTION 23u
#define VIEM_KEY_PARAGRAPH_START 24u
#define VIEM_KEY_PARAGRAPH_END 25u
#define VIEM_KEY_NEXT_PARAGRAPH 26u
/* Native GUI paste: Vim paste-before placement and cursor-after behavior.
   External text pasted into Markdown Preview is interpreted as Markdown. */
#define VIEM_KEY_PASTE_CLIPBOARD 27u
#define VIEM_KEY_ALT_ENTER 28u
/* Native Paste and Match Style: the same placement with literal text. */
#define VIEM_KEY_PASTE_CLIPBOARD_MATCHING_STYLE 29u
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
#define VIEM_EX_FRONTEND_FILE 27u
#define VIEM_EX_FRONTEND_ONLY 28u
/* Quit without writing; window_count carries the process exit status. */
#define VIEM_EX_FRONTEND_CQUIT 29u
/* Select the application theme named by text (`:colorscheme`). */
#define VIEM_EX_FRONTEND_THEME 30u
/* A Vim option mapped onto an application setting; text is `name=value` in
   `:set` spelling, Booleans as 0 or 1. */
#define VIEM_EX_FRONTEND_APPLICATION_OPTION 31u
/* Single-use acknowledgement token in window_count; always last in its batch. */
#define VIEM_EX_FRONTEND_SCRIPT_CONTINUATION 32u
/* Edit the pane's alternate file (CTRL-^); the host tracks it per pane. */
#define VIEM_EX_FRONTEND_EDIT_ALTERNATE 33u
/* Jump to an application file mark: window_command is A-Z, FORCE means linewise. */
#define VIEM_EX_FRONTEND_JUMP_FILE_MARK 34u
/* Apply startup application configuration only for the first document. */
#define VIEM_EX_FRONTEND_STARTUP_APPLICATION (1u << 15)
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
#define VIEM_EX_FRONTEND_READ_SOURCE (1u << 14)

#define VIEM_EX_OPTION_WRAP 1u
#define VIEM_EX_OPTION_NUMBER 24u
#define VIEM_EX_OPTION_LINEBREAK 2u
#define VIEM_EX_OPTION_FILE_FORMAT 3u
#define VIEM_EX_OPTION_FILE_FORMATS 4u
#define VIEM_EX_OPTION_IGNORECASE 5u
#define VIEM_EX_OPTION_SMARTCASE 6u
#define VIEM_EX_OPTION_WRAPSCAN 7u
#define VIEM_EX_OPTION_FORMATOPTIONS 25u
#define VIEM_EX_OPTION_COMMENTS 26u
#define VIEM_EX_OPTION_FORMATLISTPAT 27u
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
typedef struct ViemClipboardTurnEntry {
  uint32_t flags;
  uint32_t target;
  uint64_t generation;
  ViemUtf8Slice plain_text;
  ViemUtf8Slice fragment_json;
} ViemClipboardTurnEntry;
typedef struct ViemCommandTurnContext {
  const ViemClipboardTurnEntry *clipboards;
  uint64_t clipboard_count;
} ViemCommandTurnContext;

/* Offset and length in the UTF-8 string arena copied with an effect batch. */
typedef struct ViemEffectBytesRef {
  uint64_t offset;
  uint64_t length;
} ViemEffectBytesRef;

typedef struct ViemClipboardWrite {
  uint32_t flags;
  uint32_t target;
  uint32_t register_kind;
  uint64_t document_id;
  uint64_t document_revision;
  ViemEffectBytesRef plain_text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} ViemClipboardWrite;


/* Subordinate display value for one VIEM_EX_FRONTEND_OPTIONS request. */
typedef struct ViemExOptionDisplay {
  uint32_t name;
  uint32_t value_kind;
  uint32_t scalar_value;
  uint64_t first_file_format;
  uint64_t file_format_count;
  ViemEffectBytesRef text;
} ViemExOptionDisplay;


/* Exact resolved mark plus its captured hard-line text. */
typedef struct ViemExMark {
  uint32_t name;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  ViemEffectBytesRef line_text;
} ViemExMark;


/* Exact resolved register contents and optional semantic hard-break offsets. */
typedef struct ViemExRegister {
  uint32_t name;
  uint32_t register_kind;
  ViemEffectBytesRef text;
  uint64_t first_hard_break;
  uint64_t hard_break_count;
} ViemExRegister;


/* Exact jump in oldest-to-newest order plus its captured hard-line text. */
typedef struct ViemExJump {
  uint32_t flags;
  uint64_t list_index;
  uint64_t utf8_offset;
  uint64_t hard_line_index;
  uint64_t hard_line_start;
  uint64_t hard_line_end;
  uint64_t grapheme_column;
  ViemEffectBytesRef line_text;
} ViemExJump;


/* Exact post-turn formatted hard-line content for PRINT_LINES. */
typedef struct ViemExTextLine {
  uint64_t hard_line_index;
  uint64_t utf8_start;
  uint64_t utf8_end;
  ViemEffectBytesRef text;
} ViemExTextLine;


/*
 * One raw Ex request in execution order. text references the copied UTF-8
 * arena and is an optional path, :normal command string, or concatenated
 * Unicode mark/register-name sequence according to kind. PRINT/NORMAL ranges
 * carry inclusive zero-based addressed-unit indices. WRITE/WRITE_QUIT carry
 * exact half-open source-byte bounds. For READ, hard_line_start is an insertion
 * offset in source bytes when READ_SOURCE is set, otherwise formatted UTF-8.
 * Option indices address the option array.
 * first_payload/payload_count address the mark, register, jump, or text-line
 * array selected by the info-request kind.
 */
typedef struct ViemExFrontendRequest {
  uint32_t kind;
  uint32_t flags;
  /* One VIEM_WINDOW_* value when kind is VIEM_EX_FRONTEND_WINDOW. */
  uint32_t window_command;
  uint64_t document_id;
  uint64_t document_revision;
  ViemEffectBytesRef text;
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
  uint64_t argument_count;
  /* One-based line; zero means last line. Valid only with HAS_LINE. */
  uint64_t argument_line;
  /* Core-formatted output, independent of the typed request payload. */
  ViemEffectBytesRef display_text;
} ViemExFrontendRequest;


/* Exact immutable batch identity, flags, and two-pass copy sizes. */
typedef struct ViemEffectBatchInfo {
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
} ViemEffectBatchInfo;


typedef struct ViemTextMetrics {
  float cap_height;
  float ascent;
  float descent;
  float leading;
} ViemTextMetrics;

typedef struct ViemShapedBounds {
  float x;
  float y;
  float width;
  float height;
} ViemShapedBounds;

typedef struct ViemOpenTypeFeature {
  uint8_t tag[4];
  uint32_t value;
} ViemOpenTypeFeature;

typedef struct ViemResolvedTextStyle {
  uint32_t slant;
  uint32_t direction;
  uint32_t has_language;
  uint32_t has_script;
  /* Nonzero when weight includes relative bold (+300); retain the selected
     face as the base rather than choosing a bold face. */
  uint32_t relative_bold;
  float size;
  float weight;
  float letter_spacing;
  float baseline_offset;
  const ViemUtf8Slice *font_families;
  uint64_t font_family_count;
  ViemUtf8Slice language;
  ViemUtf8Slice script;
  const ViemOpenTypeFeature *features;
  uint64_t feature_count;
  ViemUtf8Slice font_axes; /* JSON object: four-character axis tags to finite coordinates. */
  ViemUtf8Slice font_face; /* OpenType subfamily of the primary family; empty for automatic selection. */
} ViemResolvedTextStyle;

#define VIEM_RESOLVED_TEXT_STYLE_RELATIVE_BOLD (1u << 0)


typedef struct ViemShapeStyleRun {
  uint64_t text_start;
  uint64_t text_end;
  ViemResolvedTextStyle style;
} ViemShapeStyleRun;


/*
 * Provider-owned opaque render resource. identifier is an integer token, not
 * a native pointer for core to dereference. Response arenas pin resources until
 * the next shape call. Core retains an independent fragment lease shared by
 * caches and snapshots. Tokens remain usable while leased and their generation
 * is current. Borrowed frontend exports remain tied to their exact layout.
 * Lease release remains valid after metrics invalidation or view removal.
 */
typedef struct ViemRenderRunHandle {
  uint64_t owner;
  uint64_t identifier;
  uint64_t metrics_generation;
  uint32_t threading;
} ViemRenderRunHandle;

typedef struct ViemClusterCaretStop {
  uint64_t text_offset;
  float inline_offset;
  uint32_t affinity;
} ViemClusterCaretStop;

typedef struct ViemShapedCluster {
  uint64_t text_start;
  uint64_t text_end;
  float advance;
  ViemTextMetrics metrics;
  ViemShapedBounds typographic_bounds;
  ViemShapedBounds ink_bounds;
  uint32_t bidi_level;
  uint32_t has_render_run;
  ViemUtf8Slice fallback_font;
  const ViemClusterCaretStop *caret_stops;
  uint64_t caret_stop_count;
  ViemRenderRunHandle render_run;
} ViemShapedCluster;


typedef struct ViemShapingDiagnostic {
  uint64_t text_start;
  uint64_t text_end;
  ViemUtf8Slice message;
} ViemShapingDiagnostic;


/* Passive source metadata. Never fetch an image destination. Local decoding is
 * native-owned and bounded; unavailable and remote images use a 300 x 64 point
 * placeholder. Return one cluster per object with intrinsic dimensions scaled
 * by request.scale, bottom baseline, and only endpoint caret stops. Core clamps
 * the cluster to the content width while preserving aspect ratio. Draw native
 * image resources into the final exported cluster typographic bounds. */
typedef struct ViemInlineImage {
  uint64_t text_start;
  uint64_t text_end;
  ViemUtf8Slice destination;
} ViemInlineImage;

typedef struct ViemShapeRequest {
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
  const ViemShapeStyleRun *style_runs;
  uint64_t style_run_count;
  ViemResolvedTextStyle default_style;
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
  const ViemInlineImage *inline_images;
  uint64_t inline_image_count;
} ViemShapeRequest;


typedef struct ViemShapeResponse {
  /* 0/1 native fixed-pitch trait of the resolved default-style font, independent
   * of fallback glyphs/style runs. Query native traits, never font names. */
  uint32_t default_font_is_monospaced;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  const ViemShapedCluster *clusters;
  uint64_t cluster_count;
  const uint64_t *visual_order;
  uint64_t visual_order_count;
  ViemTextMetrics default_metrics;
  const ViemShapingDiagnostic *diagnostics;
  uint64_t diagnostic_count;
} ViemShapeResponse;

/*
 * A response echoes the request's text_start..text_end ownership interior.
 * Under provider ABI v4, returned cluster ends may extend into context_after;
 * response bounds do not expand to include those cluster tails.
 */


/* One retained fragment lease. Resources stay valid until release, or a
 * metrics-generation change. Response resources are pinned until the next
 * shape_batch call. retain does not invalidate response storage. Release may
 * run on any thread and after view detach: marshal native destruction onto
 * the required executor, and do not call back into core. Both callbacks are
 * required when has_render_run_policy is true. NULL from retain is failure. */
typedef void *(*ViemRetainRenderRunsCallback)(
    void *context, const ViemRenderRunHandle *handles, uint64_t count);
typedef void (*ViemReleaseRenderRunsCallback)(void *lease);

typedef uint64_t (*ViemMetricsGenerationCallback)(void *context);
typedef uint32_t (*ViemShapeBatchCallback)(
    void *context, const ViemShapeRequest *requests, uint64_t request_count,
    ViemShapeResponse *responses, uint64_t response_capacity);

/*
 * The provider table is copied by viem_core_view_add. Only the current
 * provider ABI version is accepted. Context and callback functions remain
 * valid until the view is removed or its core is
 * successfully destroyed. VIEM_STATUS_CORE_BUSY means destruction did not
 * occur and does not end these lifetimes. Every response pointer returned by
 * shape_batch must remain readable until the next shape_batch call for that
 * view; core copies all values immediately. Retaining a fragment lease does
 * not invalidate response storage. Opaque render-run tokens follow the lease
 * and generation lifetime documented on ViemRenderRunHandle.
 *
 * A successful ABI-v4 response affirms that its ownership interior is stable
 * under arbitrary text outside the supplied bounded context. A provider that
 * cannot make that guarantee returns VIEM_STATUS_UNSTABLE_SHAPING_CONTEXT from
 * shape_batch instead of returning partial or uncacheable measurements. Core
 * then installs and caches nothing from the failed batch.
 */
typedef struct ViemTextMeasurementProvider {
  uint32_t abi_version;
  void *context;
  uint64_t measurement_environment_id;
  uint32_t threading;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  ViemMetricsGenerationCallback metrics_generation;
  ViemShapeBatchCallback shape_batch;
  ViemRetainRenderRunsCallback retain_render_runs;
  ViemReleaseRenderRunsCallback release_render_runs;
} ViemTextMeasurementProvider;


typedef struct ViemViewOptions {
  uint32_t execution_context;
  float width;
  float height;
  /* Application canvas padding, applied before the first layout. */
  float padding_top;
  float padding_left;
  float padding_bottom;
  float padding_right;
} ViemViewOptions;


/*
 * Absolute presentation origin. left is always requested. With HAS_TOP, all
 * expected_* fields must match the current immutable layout. Core maps top
 * through its compact exact/estimated height index, lays out only the local
 * viewport plus bounded overscan, and publishes both coordinates atomically.
 * The expected_* identity is ignored for a horizontal-only request.
 */
typedef struct ViemViewportOrigin {
  uint32_t flags;
  float left;
  float top;
  uint64_t expected_document_id;
  uint64_t expected_document_revision;
  uint64_t expected_layout_revision;
  uint64_t expected_configuration_generation;
  uint64_t expected_measurement_environment_id;
  uint64_t expected_metrics_generation;
} ViemViewportOrigin;

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
typedef struct ViemViewRestoration {
  uint32_t cursor_affinity;
  uint64_t cursor_line;
  uint64_t cursor_column;
  uint64_t viewport_line;
  uint64_t viewport_column;
  float row_fraction;
  float left;
} ViemViewRestoration;

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
typedef struct ViemViewportState {
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
} ViemViewportState;


/*
 * Exact identity for one immutable layout snapshot. Copy this value from a
 * snapshot-info query into every dependent geometry request. Core never
 * silently substitutes a newer layout.
 */
typedef struct ViemLayoutSnapshotIdentity {
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} ViemLayoutSnapshotIdentity;


typedef struct ViemLayoutInsets {
  float top;
  float left;
  float bottom;
  float right;
} ViemLayoutInsets;

typedef struct ViemLayoutRect {
  float x;
  float y;
  float width;
  float height;
} ViemLayoutRect;

/* Normalized RGBA components; each value is finite and in [0, 1]. */
typedef struct ViemRgba {
  float red;
  float green;
  float blue;
  float alpha;
} ViemRgba;

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
#define VIEM_STYLE_ROLE_CONTAINER 8u


#define VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION 2u
#define VIEM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY 3u

#define VIEM_STYLE_DEFINITION_HAS_PARENT (1u << 0)
#define VIEM_STYLE_DEFINITION_HAS_NEXT_STYLE (1u << 1)
#define VIEM_STYLE_DEFINITION_BASE_PARAGRAPH (1u << 3)
#define VIEM_STYLE_DEFINITION_INTERNAL (1u << 5)
#define VIEM_STYLE_DEFINITION_INTERNAL_LIST (1u << 6)
/* A generated Code syntax definition that has not been edited or persisted. */
#define VIEM_STYLE_DEFINITION_IMPLICIT (1u << 7)
/* Trusted core paragraph definitions used only by the line-number gutter. */
#define VIEM_STYLE_DEFINITION_LINE_NUMBER (1u << 8)
/* The current-line variant also carries LINE_NUMBER and INTERNAL. */
#define VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER (1u << 9)

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
#define VIEM_STYLE_PROPERTY_CHARACTER_SUPERSCRIPT 45u
#define VIEM_STYLE_PROPERTY_CHARACTER_SUBSCRIPT 46u


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
#define VIEM_STYLE_EDIT_SET_DISPLAY_NAME 7
#define VIEM_STYLE_EDIT_SET_PARENT_VARIANT 8u

typedef struct ViemStyleSheetIdentity {
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
} ViemStyleSheetIdentity;


typedef struct ViemStyleStringRef {
  uint64_t offset;
  uint64_t length;
} ViemStyleStringRef;

typedef struct ViemStyleSheetInfo {
  ViemStyleSheetIdentity identity;
  uint64_t definition_count;
  uint64_t property_count;
  uint64_t value_item_count;
  uint64_t dependency_count;
  uint64_t string_bytes;
} ViemStyleSheetInfo;


typedef struct ViemStyleValue {
  uint32_t kind;
  uint32_t enum_value;
  float number;
  ViemRgba color;
  ViemStyleStringRef string;
  uint64_t first_item;
  uint64_t item_count;
} ViemStyleValue;


typedef struct ViemStyleValueItem {
  uint32_t kind;
  ViemStyleStringRef string;
  uint32_t unsigned_value;
} ViemStyleValueItem;


typedef struct ViemStyleDependency {
  uint32_t namespace_id;
  ViemStyleStringRef style_id;
} ViemStyleDependency;


typedef struct ViemStyleDefinition {
  uint32_t flags;
  uint32_t namespace_id;
  uint32_t role;
  uint32_t origin;
  uint32_t capabilities;
  uint32_t parent_variant; /* zero for local inheritance, otherwise a source-format family */
  ViemStyleStringRef stable_id;
  ViemStyleStringRef display_name;
  ViemStyleStringRef parent_id;
  ViemStyleStringRef next_style_id;
  uint64_t first_property;
  uint64_t property_count;
} ViemStyleDefinition;


typedef struct ViemStyleProperty {
  uint32_t flags;
  uint32_t property;
  uint32_t contributor_kind;
  uint32_t contributor_namespace;
  ViemStyleValue declared;
  ViemStyleValue effective;
  ViemStyleStringRef contributor_style_id;
  uint64_t first_dependency;
  uint64_t dependency_count;
} ViemStyleProperty;


typedef struct ViemStyleEditValueItem {
  uint32_t kind;
  ViemUtf8Slice text;
  uint32_t unsigned_value;
} ViemStyleEditValueItem;


typedef struct ViemStyleEditValue {
  uint32_t kind;
  uint32_t enum_value;
  float number;
  ViemRgba color;
  ViemUtf8Slice text;
  const ViemStyleEditValueItem *items;
  uint64_t item_count;
} ViemStyleEditValue;


typedef struct ViemStyleEdit {
  uint32_t flags;
  ViemStyleSheetIdentity identity;
  uint32_t namespace_id;
  uint32_t operation;
  uint32_t property;
  ViemUtf8Slice style_id;
  ViemStyleEditValue value;
} ViemStyleEdit;


/*
 * Immutable capability for one explicit live style-edit group. token is
 * process-wide and non-reused. Every other field is also part of the
 * capability and must be returned unchanged. Begin revisions remain fixed as
 * grouped edits commit newer snapshots.
 */
typedef struct ViemStyleEditGroup {
  uint32_t flags;
  uint64_t token;
  ViemViewId view_id;
  uint64_t document_id;
  uint64_t begin_document_revision;
  uint64_t begin_style_sheet_revision;
} ViemStyleEditGroup;


#define VIEM_TEXT_PAINT_HAS_BACKGROUND (1u << 0)
#define VIEM_TEXT_PAINT_UNDERLINE (1u << 1)
#define VIEM_TEXT_PAINT_STRIKETHROUGH (1u << 2)
#define VIEM_TEXT_PAINT_DEFAULT_FOREGROUND (1u << 3)
#define VIEM_LAYOUT_PAINT_DEFAULT_CANVAS (1u << 0)

/*
 * Foreground is always present. Background is meaningful only with
 * HAS_BACKGROUND. Decoration flags mean that decoration is enabled.
 */
typedef struct ViemTextPaint {
  uint32_t flags;
  ViemRgba foreground;
  ViemRgba background;
  /* Unscaled baseline displacement; positive values raise the text. */
  float baseline_offset;
} ViemTextPaint;


/* Canvas/default paint and required override-run count for one exact layout. */
typedef struct ViemLayoutPaintInfo {
  uint32_t flags;
  ViemLayoutSnapshotIdentity identity;
  ViemRgba canvas_background;
  ViemTextPaint default_paint;
  uint64_t paint_run_count;
} ViemLayoutPaintInfo;


/* Ordered, non-overlapping logical half-open UTF-8 override range. */
typedef struct ViemPaintStyleRun {
  uint64_t text_start;
  uint64_t text_end;
  ViemTextPaint paint;
} ViemPaintStyleRun;


#define VIEM_LAYOUT_SNAPSHOT_FULL_DOCUMENT (1u << 0)
#define VIEM_LAYOUT_SNAPSHOT_PREFIX_EXACT (1u << 1)
#define VIEM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT (1u << 2)
#define VIEM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT (1u << 3)

/* Geometry is in document-layout coordinates; subtract the viewport origin. */
typedef struct ViemLayoutSnapshotInfo {
  uint32_t flags;
  ViemLayoutSnapshotIdentity identity;
  float viewport_width;
  float viewport_height;
  float usable_width;
  float content_width;
  float total_height;
  ViemLayoutInsets content_insets;
  uint64_t coverage_hard_line_start;
  uint64_t coverage_hard_line_end;
  uint64_t document_hard_line_count;
  float coverage_y_start;
  float coverage_y_end;
  uint64_t row_count;
  uint64_t cluster_count;
  uint64_t caret_count;
} ViemLayoutSnapshotInfo;


#define VIEM_VISUAL_ROW_HAS_PARAGRAPH (1u << 0)
#define VIEM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS (1u << 1)
#define VIEM_VISUAL_ROW_WRAPS_TO_NEXT (1u << 2)

/* first_* and *_count index arrays copied by the same snapshot export. */
typedef struct ViemVisualRow {
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
} ViemVisualRow;


#define VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN (1u << 0)

typedef struct ViemPositionedCluster {
  uint32_t flags;
  uint64_t row_index;
  uint64_t text_start;
  uint64_t text_end;
  float x;
  float advance;
  ViemLayoutRect typographic_bounds;
  ViemLayoutRect ink_bounds;
  uint32_t bidi_level;
  ViemRenderRunHandle render_run;
} ViemPositionedCluster;


#define VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER (1u << 1)
#define VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND (1u << 2)
#define VIEM_LAYOUT_DECORATION_BLOCK_BORDER (1u << 3)
#define VIEM_LAYOUT_DECORATION_CODE_LANGUAGE (1u << 4)
/* Fixed language label: no picker interaction or dropdown indicator. */
#define VIEM_LAYOUT_DECORATION_LANGUAGE_PICKER_DISABLED (1u << 5)
#define VIEM_LAYOUT_DECORATION_CHECKBOX (1u << 6)
#define VIEM_LAYOUT_DECORATION_CHECKED (1u << 7)
/* GitHub alert furniture in its title's color: a vector icon filling its
 * square bounds, and title clusters shaped as a Heading 4 inside the alert,
 * drawn from their render runs at the decoration baseline. Both carry their
 * VIEM_ALERT_* kind. */
#define VIEM_LAYOUT_DECORATION_ALERT_ICON (1u << 8)
#define VIEM_LAYOUT_DECORATION_ALERT_TITLE (1u << 9)
#define VIEM_LAYOUT_DECORATION_ALERT_KIND_SHIFT 10u
#define VIEM_LAYOUT_DECORATION_ALERT_KIND_MASK (7u << 10)
/* Diff mode's divider for collapsed unchanged lines: a band (with
 * BLOCK_BACKGROUND) or one of its top and bottom rules (with BLOCK_BORDER),
 * painted like those, across the whole row width of its paragraph's first or
 * last row. */
#define VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT (1u << 13)
/* Noneditable block furniture; label offsets address only the separate label
 * byte blob. No decoration creates formatted offsets, caret or selection stops. */
typedef struct ViemLayoutDecoration {
  uint64_t paragraph_id;
  uint32_t flags;
  uint64_t row_index;
  uint64_t label_byte_start;
  uint64_t label_byte_length;
  float x;
  float advance;
  float font_size;
  /* A text decoration's layout baseline: its row's, except an alert title in
   * the band above its row. Zero for non-text furniture. */
  float baseline;
  ViemLayoutRect typographic_bounds;
  ViemLayoutRect ink_bounds;
  ViemRenderRunHandle render_run;
  ViemTextPaint paint;
} ViemLayoutDecoration;
typedef struct ViemLayoutDecorationsInfo {
  ViemLayoutSnapshotIdentity identity;
  uint64_t decoration_count;
  uint64_t label_bytes;
} ViemLayoutDecorationsInfo;

typedef struct ViemPositionedCaret {
  uint32_t affinity;
  uint64_t row_index;
  uint64_t text_offset;
  float x;
} ViemPositionedCaret;


typedef struct ViemLayoutCaretRequest {
  uint32_t affinity;
  ViemLayoutSnapshotIdentity identity;
  uint64_t text_offset;
} ViemLayoutCaretRequest;


/* Character-cell hit testing for pointer-down in character-addressing modes. */
#define VIEM_LAYOUT_HIT_TEST_POINTER_DOWN (1u << 0)

typedef struct ViemLayoutHitTestRequest {
  uint32_t flags;
  ViemLayoutSnapshotIdentity identity;
  float x;
  float y;
} ViemLayoutHitTestRequest;


typedef struct ViemLayoutCaretPoint {
  uint32_t affinity;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t text_offset;
} ViemLayoutCaretPoint;


#define VIEM_CARET_GEOMETRY_CLUSTER_FALLBACK (1u << 0)

typedef struct ViemLayoutCaretGeometry {
  uint32_t flags;
  ViemLayoutCaretPoint point;
  ViemLayoutRect rect;
  uint64_t row_index;
} ViemLayoutCaretGeometry;


/* Predicted plain-click caret; CURRENT means it already occupies this target. */
#define VIEM_POINTER_CARET_CURRENT (1u << 0)
typedef struct ViemPointerCaret {
  uint32_t flags;
  uint32_t mode;
  uint32_t caret_shape;
  uint32_t affinity;
  float font_en_width;
  uint64_t text_start;
  uint64_t text_end;
} ViemPointerCaret;

/* Read-only plain-click prediction. Request flags must be zero. */
ViemStatus viem_core_view_pointer_caret(ViemCoreHandle handle, ViemViewId view,
    const ViemLayoutHitTestRequest *request, ViemPointerCaret *out_caret);

#define VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR (1u << 0)
#define VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT (1u << 1)
#define VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK (1u << 2)
#define VIEM_VIEW_PRESENTATION_HAS_COMMAND_LINE (1u << 3)
#define VIEM_VIEW_PRESENTATION_HAS_DESIRED_X (1u << 4)
/* Route the next input to the core before native editing shortcuts. */
#define VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING (1u << 5)
/* Route prompt register selectors to core, including native text events. */
#define VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING (1u << 6)
/* Pending Vim grammar owns input before native formatting accelerators. */
#define VIEM_VIEW_PRESENTATION_COMMAND_INPUT_PENDING (1u << 7)

/*
 * Linear Visual anchors have no retained visual affinity, so the anchor
 * affinity is zero unless VISUAL_ANCHOR_AFFINITY_EXACT is present. Visual
 * Block endpoints retain exact affinity and display-space x edges.
 */
typedef struct ViemViewPresentation {
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
  uint64_t command_line_utf8_length;
  uint64_t command_line_cursor_utf8_offset;
  /* Exact formatted range the caret occupies. A cell covers one grapheme of
     hard-line content; a boundary is empty, with both ends at the caret. */
  uint64_t caret_utf8_start;
  uint64_t caret_utf8_end;
  /* Zero register/count means inactive; pending text is bounded UTF-8 with an
   * ellipsis when truncated. These values are presentation, never input. */
  uint32_t recording_register;
  uint32_t pending_command_length;
  uint64_t active_count;
  uint8_t pending_command[32];
} ViemViewPresentation;


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
#define VIEM_COMMAND_LINE_KIND_EXPRESSION 4u

/*
 * state_identity is an opaque exact-content token. It changes whenever the
 * exported kind, UTF-8 cursor, or bytes change without a document revision.
 */
typedef struct ViemCommandLineIdentity {
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint8_t state_identity[32];
} ViemCommandLineIdentity;


/* The prompt prefix is represented by identity.kind, not by the UTF-8 bytes. */
typedef struct ViemCommandLineInfo {
  ViemCommandLineIdentity identity;
  uint64_t utf8_length;
  uint64_t cursor_utf8_offset;
} ViemCommandLineInfo;


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
typedef struct ViemCompletionInfo {
  uint32_t flags;
  uint64_t session_id;
  uint64_t generation;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t view_id;
  int64_t selected_index;
  uint64_t item_count;
  uint64_t utf8_length;
  ViemLayoutSnapshotIdentity anchor_layout;
  ViemLayoutRect anchor_rect;
} ViemCompletionInfo;


/* Byte ranges in the concatenated, length-delimited UTF-8 candidate arena. */
typedef struct ViemCompletionItem {
  uint64_t text_offset;
  uint64_t text_length;
} ViemCompletionItem;


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
typedef struct ViemVisualSelectionIdentity {
  uint32_t kind;
  ViemLayoutSnapshotIdentity layout;
  uint8_t state_identity[32];
} ViemVisualSelectionIdentity;


/*
 * Segments are in logical UTF-8 document order. Character/Line segments retain
 * the entire selected range, including text outside materialized layout.
 * Rectangles cover only materialized geometry, in visual-row, then x order.
 */
typedef struct ViemVisualSelectionInfo {
  ViemVisualSelectionIdentity identity;
  uint64_t segment_count;
  uint64_t rectangle_count;
} ViemVisualSelectionInfo;


/*
 * One logical half-open UTF-8 range. Character/Line segments have zero flags.
 * Block segments retain their visual row, hard line, and display-left/right
 * affinities; these affinities are not logical start/end labels in bidi text.
 */
typedef struct ViemVisualSelectionSegment {
  uint32_t flags;
  uint64_t text_start;
  uint64_t text_end;
  uint64_t row_index;
  uint64_t hard_line_index;
  uint32_t left_affinity;
  uint32_t right_affinity;
} ViemVisualSelectionSegment;


/* segment_index indexes the segment array copied by the same call. */
typedef struct ViemVisualSelectionRectangle {
  uint64_t row_index;
  uint64_t segment_index;
  ViemLayoutRect rect;
} ViemVisualSelectionRectangle;


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
/* Normal/Insert/Replace caret; changes pending typing declarations, not source.
 * A character control in Normal starts Insert with the requested declaration. */
#define VIEM_SEMANTIC_STYLE_TYPING_CONTEXT (1u << 3)

/*
 * Exact logical selection identity with no layout dependency. Only Character
 * and Line selections name an actionable contiguous range in this ABI.
 */
typedef struct ViemLogicalSelectionIdentity {
  uint32_t kind;
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t text_start;
  uint64_t text_end;
  uint8_t state_identity[32];
} ViemLogicalSelectionIdentity;


#define VIEM_TABLE_CAN_INSERT 1u
#define VIEM_TABLE_IN_TABLE 2u
#define VIEM_TABLE_HEADER 4u
#define VIEM_TABLE_CELL_SELECTED 8u
/* Cursor contexts only: the view's cell selection covers complete rows and/or
 * complete columns. row..row_end and column..column_end then name the selected
 * group, and row, column and delete actions of that kind apply to the group.
 * Insertions go outside the group and keep it selected. Alignment applies to
 * every column of a column group and keeps it selected. Deleting rows that
 * include the header but not every row clears the header instead of deleting
 * it, with or without a group. */
#define VIEM_TABLE_ROW_SELECTION 16u
#define VIEM_TABLE_COLUMN_SELECTION 32u
#define VIEM_TABLE_ALIGN_UNSPECIFIED 0u
#define VIEM_TABLE_ALIGN_LEFT 1u
#define VIEM_TABLE_ALIGN_CENTER 2u
#define VIEM_TABLE_ALIGN_RIGHT 3u
/* Context only: the target columns differ. Unspecified and left together report
 * left. Setting an alignment applies it to every target column. */
#define VIEM_TABLE_ALIGN_MIXED 4u
#define VIEM_TABLE_INSERT_ROW_ABOVE 1u
#define VIEM_TABLE_INSERT_ROW_BELOW 2u
#define VIEM_TABLE_DELETE_ROW 3u
#define VIEM_TABLE_INSERT_COLUMN_LEFT 4u
#define VIEM_TABLE_INSERT_COLUMN_RIGHT 5u
#define VIEM_TABLE_DELETE_COLUMN 6u
#define VIEM_TABLE_SET_ALIGNMENT 7u
#define VIEM_TABLE_CLEAR_CELL 8u

typedef struct ViemTableContext {
  uint32_t flags;
  ViemLogicalSelectionIdentity selection;
  uint64_t table_id;
  uint64_t row;
  uint64_t column;
  uint64_t rows;
  uint64_t columns;
  /* Exclusive ends of the target; row + 1 and column + 1 for one cell. */
  uint64_t row_end;
  uint64_t column_end;
  uint32_t alignment;
} ViemTableContext;

typedef struct ViemInsertTable {
  ViemLogicalSelectionIdentity expected_selection;
  uint32_t columns;
  uint32_t body_rows;
} ViemInsertTable;

typedef struct ViemTableAction {
  uint32_t action;
  ViemTableContext expected;
  uint32_t alignment;
} ViemTableAction;

/* Current WYSIWYG cell geometry; rects are in document layout coordinates.
 * Exported only from the exact requested immutable layout snapshot. */
typedef struct ViemTableCell {
  uint64_t table_id;
  uint64_t row;
  uint64_t column;
  uint64_t cell_id;
  uint64_t text_start;
  uint64_t text_end;
  ViemLayoutRect rect;
  ViemLayoutRect table_rect;
  uint32_t alignment;
  uint32_t flags;
} ViemTableCell;

typedef struct ViemTableSelection {
  uint32_t active;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t table_id;
  uint64_t anchor_row;
  uint64_t anchor_column;
  uint64_t active_row;
  uint64_t active_column;
} ViemTableSelection;

/* Adapter capability and check/mixed state at one core-owned selection. */
typedef struct ViemSemanticStylePresentation {
  uint32_t style;
  uint32_t state;
  uint32_t flags;
  ViemLogicalSelectionIdentity selection;
} ViemSemanticStylePresentation;


/* Exact request using an identity returned by the presentation query. */
typedef struct ViemSetSemanticStyle {
  uint32_t style;
  uint32_t enabled;
  ViemLogicalSelectionIdentity expected_selection;
} ViemSetSemanticStyle;


#define VIEM_PLACE_CURSOR_EXTEND_SELECTION (1u << 0)
/* Select whole words; EXTEND retains the gesture's original word. */
#define VIEM_PLACE_CURSOR_WORD_SELECTION (1u << 1)
/* Seed a character-inclusive drag origin; incompatible with extend/word flags. */
#define VIEM_PLACE_CURSOR_BEGIN_POINTER_GESTURE (1u << 2)

typedef struct ViemPlaceCursor {
  uint32_t flags;
  uint64_t document_revision;
  uint64_t text_offset;
  uint32_t affinity;
} ViemPlaceCursor;


/* Concrete file-format target bound to one exact document snapshot. */
typedef struct ViemSetFileFormat {
  uint32_t file_format;
  uint64_t document_id;
  uint64_t document_revision;
} ViemSetFileFormat;


typedef struct ViemSetEncoding {
  uint32_t encoding;
  uint64_t document_id;
  uint64_t document_revision;
} ViemSetEncoding;

#define VIEM_LIST_STYLE_NONE 0u
#define VIEM_LIST_STYLE_BULLET 1u
#define VIEM_LIST_STYLE_NUMBERED 2u
#define VIEM_LIST_STYLE_TASK 3u
typedef struct ViemSetListStyle {
  uint32_t style;
  ViemLogicalSelectionIdentity expected_selection;
} ViemSetListStyle;

typedef struct ViemSetBlockQuote {
  uint32_t enabled;
  ViemLogicalSelectionIdentity expected_selection;
} ViemSetBlockQuote;

/* GitHub alert kinds; VIEM_ALERT_NONE makes a plain quote. */
#define VIEM_ALERT_NONE 0u
#define VIEM_ALERT_NOTE 1u
#define VIEM_ALERT_TIP 2u
#define VIEM_ALERT_IMPORTANT 3u
#define VIEM_ALERT_WARNING 4u
#define VIEM_ALERT_CAUTION 5u
typedef struct ViemSetAlert {
  uint32_t kind;
  ViemLogicalSelectionIdentity expected_selection;
} ViemSetAlert;

#define VIEM_LIST_CAN_INDENT 1u
#define VIEM_LIST_CAN_UNINDENT 2u
#define VIEM_LIST_CAN_INSERT_PARAGRAPH 4u
typedef struct ViemListIndent {
  uint32_t unindent;
  ViemLogicalSelectionIdentity expected_selection;
} ViemListIndent;

typedef struct ViemSetParagraphStyle {
  uint32_t level; /* 0 = Base Paragraph, 1..6 = heading. */
  ViemLogicalSelectionIdentity expected_selection;
} ViemSetParagraphStyle;

/* Named paragraph or character style assignment. With no selection a character
 * assignment updates subsequent typing and enters Insert from Normal. An empty character style_id clears the
 * named assignment (Default Paragraph); it is never a stored style definition.
 * Paragraph assignment accepts the current paragraph identity. */
typedef struct ViemAssignStyle {
  uint32_t namespace;
  ViemStyleSheetIdentity identity;
  ViemLogicalSelectionIdentity expected_selection;
  ViemUtf8Slice style_id;
} ViemAssignStyle;

/* New sparse Code style definition. Empty parent uses the namespace base;
 * empty next style leaves it absent. */
typedef struct ViemCreateStyle {
  uint32_t namespace;
  ViemStyleSheetIdentity identity;
  ViemUtf8Slice style_id;
  ViemUtf8Slice display_name;
  ViemUtf8Slice parent_id;
  ViemUtf8Slice next_style_id;
} ViemCreateStyle;

typedef struct ViemDeleteStyle {
  uint32_t namespace;
  ViemStyleSheetIdentity identity;
  ViemUtf8Slice style_id;
} ViemDeleteStyle;

/* Exact acknowledgement of a successfully persisted native snapshot. */
typedef struct ViemMarkSaved {
  uint64_t document_id;
  uint64_t document_revision;
} ViemMarkSaved;


typedef struct ViemKeyInput {
  uint32_t kind;
  /* Unicode scalar, or function-key number 1..35 for VIEM_KEY_FUNCTION. */
  uint32_t codepoint;
  /* VIEM_KEY_MODIFIER_* bits for function and navigation keys; zero otherwise. */
  uint32_t modifiers;
} ViemKeyInput;


/*
 * Exact formatted-snapshot replacement target for native marked text. The
 * frontend passes its current selection, or an empty range at its insertion
 * caret, using UTF-8 byte offsets for document_revision.
 */
typedef struct ViemCompositionBegin {
  uint64_t document_revision;
  uint64_t replacement_start;
  uint64_t replacement_end;
} ViemCompositionBegin;


/* selected_start/end are UTF-8 byte offsets relative to marked_text. */
typedef struct ViemCompositionUpdate {
  uint64_t document_revision;
  ViemUtf8Slice marked_text;
  uint64_t selected_start;
  uint64_t selected_end;
} ViemCompositionUpdate;


typedef struct ViemCompositionCommit {
  uint64_t document_revision;
  ViemUtf8Slice committed_text;
} ViemCompositionCommit;


typedef struct ViemCompositionCancel {
  uint64_t document_revision;
} ViemCompositionCancel;


#define VIEM_COMPOSITION_OVERLAY_ACTIVE (1u << 0)

typedef struct ViemCompositionOverlayIdentity {
  uint64_t view_id;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t generation;
} ViemCompositionOverlayIdentity;


/* Exact ranges in the disposable, source-nonmutating composed projection. */
typedef struct ViemCompositionOverlayInfo {
  uint32_t flags;
  ViemCompositionOverlayIdentity identity;
  uint64_t utf8_length;
  uint64_t replacement_start;
  uint64_t replacement_end;
  uint64_t marked_start;
  uint64_t marked_end;
  uint64_t selected_start;
  uint64_t selected_end;
} ViemCompositionOverlayInfo;


typedef struct ViemCompositionOverlayUtf8Range {
  ViemCompositionOverlayIdentity identity;
  uint64_t start;
  uint64_t end;
} ViemCompositionOverlayUtf8Range;


typedef struct ViemCoreOutcome {
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
} ViemCoreOutcome;


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
                                    ViemDocumentState *out_state);

/*
 * Bounded access to the immutable formatted projection. Snapshot info returns
 * the exact identity used by all dependent operations. The range and mapping
 * copies are all-or-none: a short output returns BUFFER_TOO_SMALL and the
 * complete required byte/element count without writing any output item.
 * Invalid scalar/surrogate boundaries have distinct typed statuses, and an
 * identity mismatch returns STALE_REVISION.
 */
ViemStatus viem_core_formatted_snapshot_info(
    ViemCoreHandle core, ViemFormattedSnapshotInfo *out_info);
ViemStatus viem_core_copy_formatted_utf8_range(
    ViemCoreHandle core, const ViemFormattedUtf8Range *request,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_map_formatted_utf8_to_utf16(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentity *identity,
    const uint64_t *utf8_offsets, uint64_t offset_count,
    uint64_t *utf16_offsets, uint64_t output_capacity,
    uint64_t *out_required);
ViemStatus viem_core_map_formatted_utf16_to_utf8(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentity *identity,
    const uint64_t *utf16_offsets, uint64_t offset_count,
    uint64_t *utf8_offsets, uint64_t output_capacity,
    uint64_t *out_required);
ViemStatus viem_core_formatted_point_info(
    ViemCoreHandle core, const ViemFormattedSnapshotIdentity *identity,
    uint64_t utf8_offset, ViemFormattedPointInfo *out_info);

/* Snapshot-checked buffer policies; they do not modify source or undo state. */
ViemStatus viem_core_set_read_only(ViemCoreHandle core, uint64_t expected_document, uint64_t expected_revision, uint32_t read_only);
ViemStatus viem_core_mark_recovered(ViemCoreHandle core, uint64_t expected_document, uint64_t expected_revision);

/* Call only after the native write of this exact source snapshot succeeds. */
ViemStatus viem_core_mark_saved(ViemCoreHandle core,
                                const ViemMarkSaved *request);

/*
 * Query/copy one exact immutable normalized style sheet. The copy is all-or-
 * none across every typed array and the UTF-8 string arena. A zero-capacity
 * call returns BUFFER_TOO_SMALL plus exact current sizes. Any identity change
 * returns STALE_REVISION rather than substituting a newer sheet.
 */
/* Identity only: does not resolve or serialize styles. core=0 selects Code. */
ViemStatus viem_core_style_sheet_identity(ViemCoreHandle core,
                                          ViemStyleSheetIdentity *out_identity);
/* gutter_underlay=0 exports the full sheet. VIEM_LINE_NUMBER_DIFF_ADDED
 * or REMOVED exports only the two gutter styles over that diff line. */
ViemStatus viem_core_style_sheet_info(ViemCoreHandle core, uint32_t gutter_underlay,
                                      ViemStyleSheetInfo *out_info);
ViemStatus viem_core_copy_style_sheet(
    ViemCoreHandle core, uint32_t gutter_underlay, const ViemStyleSheetIdentity *expected,
    ViemStyleDefinition *definitions, uint64_t definition_capacity,
    ViemStyleProperty *properties, uint64_t property_capacity,
    ViemStyleValueItem *value_items, uint64_t value_item_capacity,
    ViemStyleDependency *dependencies, uint64_t dependency_capacity,
    uint8_t *string_bytes, uint64_t string_capacity,
    ViemStyleSheetInfo *out_info);

ViemStatus viem_core_view_add(
    ViemCoreHandle core, const ViemViewOptions *options,
    const ViemTextMeasurementProvider *provider, ViemViewId *out_view,
    ViemCoreOutcome *out_outcome);
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
    const ViemTextMeasurementProvider *provider, uint64_t *out_result);
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
                                ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_capture_restoration(ViemCoreHandle core, ViemViewId view,
    ViemViewRestoration *out_state);
/* Stage positions in the explicitly identified replacement before publication.
 * Input and output must be aligned, readable/writable and mutually disjoint. */
ViemStatus viem_core_view_restore(ViemCoreHandle core, ViemViewId view,
    uint64_t document, uint64_t revision, const ViemViewRestoration *state,
    ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_viewport_state(ViemCoreHandle core, ViemViewId view,
                                         ViemViewportState *out_state);

/*
 * Snapshot info is an atomic current-layout query. To copy geometry, pass its
 * exact identity to copy_layout_snapshot. Null array pointers are accepted
 * only with zero capacities; BUFFER_TOO_SMALL writes required counts to
 * out_info and writes no array elements. Render-run tokens are borrowed under
 * their provider generation/view lifetime and are not retained by this call.
 */
ViemStatus viem_core_view_layout_snapshot_info(
    ViemCoreHandle core, ViemViewId view, ViemLayoutSnapshotInfo *out_info);
ViemStatus viem_core_view_copy_layout_snapshot(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected,
    ViemVisualRow *rows, uint64_t row_capacity,
    ViemPositionedCluster *clusters, uint64_t cluster_capacity,
    ViemPositionedCaret *carets, uint64_t caret_capacity,
    ViemLayoutSnapshotInfo *out_info);

/*
 * Paint info is an atomic query against the current immutable layout. Pass its
 * exact identity to copy_layout_paint. Null run output is accepted only with
 * zero capacity; BUFFER_TOO_SMALL writes the required count and fixed paint
 * state to out_info and writes no run elements.
 */
ViemStatus viem_core_view_layout_paint_info(
    ViemCoreHandle core, ViemViewId view, ViemLayoutPaintInfo *out_info);
/* Exact-layout, atomic decorations export; zero-capacity buffers query counts. */
ViemStatus viem_core_view_copy_layout_decorations(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected,
    ViemLayoutDecoration *decorations, uint64_t decoration_capacity,
    uint8_t *labels, uint64_t label_capacity,
    ViemLayoutDecorationsInfo *out_info);
ViemStatus viem_core_view_copy_layout_paint(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected,
    ViemPaintStyleRun *runs, uint64_t run_capacity,
    ViemLayoutPaintInfo *out_info);

ViemStatus viem_core_view_caret_geometry(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutCaretRequest *request,
    ViemLayoutCaretGeometry *out_geometry);
ViemStatus viem_core_view_layout_hit_test(
    ViemCoreHandle core, ViemViewId view,
    const ViemLayoutHitTestRequest *request,
    ViemLayoutCaretPoint *out_point);
ViemStatus viem_core_view_presentation(
    ViemCoreHandle core, ViemViewId view,
    ViemViewPresentation *out_presentation);

/*
 * Command-line info is an atomic current-state query. Copy requires its exact
 * identity. Null UTF-8 output is accepted only with zero capacity;
 * BUFFER_TOO_SMALL writes the current required length and no bytes.
 */
ViemStatus viem_core_view_command_line_info(
    ViemCoreHandle core, ViemViewId view, ViemCommandLineInfo *out_info);
ViemStatus viem_core_view_copy_command_line(
    ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentity *expected,
    uint8_t *utf8, uint64_t utf8_capacity,
    ViemCommandLineInfo *out_info);

/*
 * Copy requires an unchanged info record from the atomic current-state query.
 * Any changed session, generation, document revision, or presentation field
 * returns STALE_REVISION. Null output is accepted only with zero capacity.
 * BUFFER_TOO_SMALL writes the required count and no output elements/bytes;
 * other failures clear the count after pointer/record validation. Each call's
 * input, output, and count regions must be aligned and pairwise disjoint.
 */
ViemStatus viem_core_view_completion_info(
    ViemCoreHandle core, ViemViewId view, ViemCompletionInfo *out_info);
ViemStatus viem_core_view_copy_completion_items(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompletionInfo *expected,
    ViemCompletionItem *items, uint64_t capacity, uint64_t *out_count);
ViemStatus viem_core_view_copy_completion_utf8(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompletionInfo *expected,
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
    ViemVisualSelectionInfo *out_info);
ViemStatus viem_core_view_copy_visual_selection(
    ViemCoreHandle core, ViemViewId view,
    const ViemVisualSelectionIdentity *expected,
    ViemVisualSelectionSegment *segments, uint64_t segment_capacity,
    ViemVisualSelectionRectangle *rectangles, uint64_t rectangle_capacity,
    ViemVisualSelectionInfo *out_info);

/*
 * Query source-backed Strong/Emphasis state and capability using only the
 * current core-owned logical selection. No layout snapshot is required.
 */
ViemStatus viem_core_view_semantic_style_presentation(
    ViemCoreHandle core, ViemViewId view, uint32_t style,
    ViemSemanticStylePresentation *out_presentation);

/* Apply to the exact logical-selection identity returned by the query. */
ViemStatus viem_core_view_set_semantic_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetSemanticStyle *request,
    ViemCoreOutcome *out_outcome);

/*
 * Bind an exact current Visual selection as the literal forward-search target.
 * The selection identity includes both layout and opaque controller state;
 * stale requests fail rather than capturing different text.
 */
ViemStatus viem_core_view_use_selection_for_find(
    ViemCoreHandle core, ViemViewId view,
    const ViemVisualSelectionIdentity *expected,
    ViemCoreOutcome *out_outcome);

/* Reveal the active endpoint of the core-owned Visual selection. */
ViemStatus viem_core_view_reveal_selection(
    ViemCoreHandle core, ViemViewId view,
    ViemCoreOutcome *out_outcome);

/*
 * Host-context turns consume immutable per-turn clipboard snapshots and
 * writable capabilities. On success, out_effect_batch is zero or an owned
 * handle which must be released. The effect batch survives core destruction.
 */
ViemStatus viem_core_view_send_key_with_host_context(
    ViemCoreHandle handle, ViemViewId view, const ViemKeyInput *input,
    const ViemCommandTurnContext *context, ViemCoreOutcome *out_outcome,
    ViemEffectBatchHandle *out_effect_batch);
ViemStatus viem_core_view_send_text_with_host_context(
    ViemCoreHandle handle, ViemViewId view, const uint8_t *text, uint64_t text_length,
    const ViemCommandTurnContext *context, ViemCoreOutcome *out_outcome,
    ViemEffectBatchHandle *out_effect_batch);

/* Pending multi-key mapping state and timeout dispatch. Flush captures host
 * context and returns an owned effect batch just like send_key. */
ViemStatus viem_core_view_has_pending_mapping(ViemCoreHandle core, ViemViewId view,
                                             uint8_t *pending);
ViemStatus viem_core_view_flush_mapping_with_host_context(
    ViemCoreHandle core, ViemViewId view, const ViemCommandTurnContext *context,
    ViemCoreOutcome *outcome, ViemEffectBatchHandle *effects);

ViemStatus viem_effect_batch_info(ViemEffectBatchHandle batch,
                                  ViemEffectBatchInfo *out_info);
/* One read-only descriptor; pointers remain valid until batch release.
 * The caller must keep the handle alive while reading; concurrent release is
 * prohibited. Copy any data needed after release. Zero-count arrays are empty.
 * Raw file requests perform no I/O and do not establish a save point. */
typedef struct ViemEffectBatchView {
  ViemEffectBatchInfo info;
  const ViemClipboardWrite *clipboard_writes;
  const ViemExFrontendRequest *ex_requests;
  const ViemExOptionDisplay *ex_options;
  const ViemExMark *ex_marks;
  const ViemExRegister *ex_registers;
  const ViemExJump *ex_jumps;
  const ViemExTextLine *ex_text_lines;
  const uint32_t *file_formats;
  const uint64_t *hard_breaks;
  const uint8_t *strings;
} ViemEffectBatchView;
ViemStatus viem_effect_batch_read(ViemEffectBatchHandle batch,
                                ViemEffectBatchView *out_batch);
ViemStatus viem_effect_batch_release(ViemEffectBatchHandle batch);

ViemStatus viem_core_view_place_cursor(
    ViemCoreHandle core, ViemViewId view,
    const ViemPlaceCursor *request, ViemCoreOutcome *out_outcome);

/* Select all logical content through EOF, independently of line policy. */
ViemStatus viem_core_view_select_all(
    ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision,
    ViemCoreOutcome *out_outcome);

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
#define VIEM_NATIVE_SELECT_WORD 1
#define VIEM_NATIVE_SELECT_SENTENCE 2
#define VIEM_NATIVE_SELECT_PARAGRAPH 3
#define VIEM_NATIVE_SELECT_LINE 4
#define VIEM_NATIVE_SELECT_VISUAL_ROW 5
#define VIEM_NATIVE_SELECT_BLOCK 6
#define VIEM_NATIVE_FIND 7
#define VIEM_NATIVE_FIND_REPLACE 8
#define VIEM_NATIVE_FIND_NEXT 9
#define VIEM_NATIVE_FIND_PREVIOUS 10
/* Availability uses bit 1 << action. return_mode=0 infers the initiating mode;
 * INSERT/REPLACE resume typing and all other valid modes resume NORMAL. */
ViemStatus viem_core_view_native_actions(ViemCoreHandle core, ViemViewId view, uint64_t *out_actions);
ViemStatus viem_core_view_native_action(ViemCoreHandle core, ViemViewId view, uint32_t action,
    uint32_t origin, uint32_t return_mode, ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effects);

ViemStatus viem_core_view_set_selection_origin(
    ViemCoreHandle core, ViemViewId view, uint32_t origin, uint32_t return_mode,
    ViemCoreOutcome *out_outcome);

/* Enter Normal mode and reveal the first nonblank grapheme on a physical source
 * line in the exact document revision. Lines are one-based; zero selects the
 * first and excess (including UINT64_MAX) selects the last. Pending input is
 * cancelled without executing it, changing source, or recording a macro key. */
ViemStatus viem_core_view_go_to_line(
    ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision, uint64_t line,
    ViemCoreOutcome *out_outcome);

/* Session file marks are bounded to A-Z. Query generation zero, then use the
 * returned generation for path copying and navigation. Outputs are disjoint. */
typedef struct ViemFileMarkTarget {
    uint32_t name;
    uint64_t generation;
    uint64_t document_id;
    uint64_t path_length;
} ViemFileMarkTarget;
ViemStatus viem_file_mark_target(uint32_t name, uint64_t generation,
    ViemFileMarkTarget *out_info, uint8_t *path, uint64_t capacity);
ViemStatus viem_core_view_jump_file_mark(ViemCoreHandle core, ViemViewId view,
    uint32_t name, uint64_t generation, uint32_t linewise, ViemCoreOutcome *out_outcome);
/* Exact UTF-8 input, copied before return; # is view-local, ~ is an accepted drop. */
ViemStatus viem_core_view_set_host_register(ViemCoreHandle core, ViemViewId view,
    uint32_t name, const uint8_t *text, uint64_t length);

/* Explicit native drop; all regions disjoint, ownership of returned effects
 * follows other host-context turns. Successful drops publish the ~ register. */
ViemStatus viem_core_view_drop_text(ViemCoreHandle core, ViemViewId view,
    const uint8_t *text, uint64_t length, const ViemCommandTurnContext *context,
    ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effect_batch);

/* Native Edit-menu history navigation; behavior is independent of Vim mode. */
ViemStatus viem_core_view_undo(ViemCoreHandle core, ViemViewId view,
                               ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_redo(ViemCoreHandle core, ViemViewId view,
                               ViemCoreOutcome *out_outcome);

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
    const ViemCompositionBegin *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_composition_update(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionUpdate *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_composition_overlay_info(
    ViemCoreHandle core, ViemViewId view,
    ViemCompositionOverlayInfo *out_info);
ViemStatus viem_core_view_copy_composition_utf8_range(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionOverlayUtf8Range *request, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_composition_commit(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionCommit *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_composition_cancel(
    ViemCoreHandle core, ViemViewId view,
    const ViemCompositionCancel *request, ViemCoreOutcome *out_outcome);

/*
 * Horizontal-only origin changes do not shape, reflow, or replace immutable
 * layout. HAS_TOP validates the supplied identity and atomically installs one
 * bounded regional viewport; a stale identity changes no view state.
 */
ViemStatus viem_core_view_set_viewport_origin(
    ViemCoreHandle core, ViemViewId view,
    const ViemViewportOrigin *request, ViemCoreOutcome *out_outcome);

/* Theme padding is presentation-only and scrolls with the document canvas. */
ViemStatus viem_core_view_set_padding(ViemCoreHandle core, ViemViewId view,
    float top, float left, float bottom, float right);
/* Notify one published image resource generation before refreshing the view.
 * The provider has advanced once from previous_metrics_generation. Destinations
 * names images whose intrinsic dimensions changed (at most 256 UTF-8 slices,
 * total at most 1 MiB). An empty array means pixels/status-only changes and
 * preserves exact measured heights. Geometry changes keep old numeric heights
 * as estimates until each requested region is measured. No resource I/O occurs. */
ViemStatus viem_core_image_resources_changed(ViemCoreHandle core, ViemViewId view,
                                             uint64_t previous_metrics_generation,
                                             const ViemUtf8Slice *destinations,
                                             uint64_t count);

ViemStatus viem_core_view_resize(ViemCoreHandle core, ViemViewId view,
                                 float width, float height,
                                 ViemCoreOutcome *out_outcome);
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
                                    ViemCoreOutcome *out_outcome);
#define VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT 1u
#define VIEM_LINE_LOCATION_FRAGMENT_EXACT 2u
typedef struct ViemViewLineLocation {
    uint32_t mode;
    uint32_t flags;
    uint64_t line;
    uint64_t column;
    uint64_t hard_line;
    uint64_t fragment;
} ViemViewLineLocation;
/* Exact physical source coordinates regardless of command line mode; no layout is forced. */
ViemStatus viem_core_view_line_location(ViemCoreHandle handle, ViemViewId view, ViemViewLineLocation *out_location);


#define VIEM_LINE_NUMBER_SNAPSHOT_ENABLED 1u
#define VIEM_LINE_NUMBER_REGION_ESTIMATED 1u
/* This displayed representative owns the visual row containing the caret. */
#define VIEM_LINE_NUMBER_REGION_CURRENT (1u << 1)
#define VIEM_LINE_NUMBER_DIFF_ADDED (1u << 2)
#define VIEM_LINE_NUMBER_DIFF_REMOVED (1u << 3)
typedef struct ViemLineNumberSnapshotInfo {
  uint32_t flags;
  ViemLayoutSnapshotIdentity identity;
  ViemLayoutRect viewport;
  uint64_t source_line_count;
  uint64_t current_source_line;
  uint64_t region_count;
} ViemLineNumberSnapshotInfo;
typedef struct ViemLineNumberRegion {
  uint64_t source_line;
  uint32_t flags;
  float top;
  float bottom;
} ViemLineNumberRegion;
/* View-local native preference. It changes no source, history or layout. */
ViemStatus viem_core_view_number(ViemCoreHandle core, ViemViewId view, uint32_t *out_enabled);
ViemStatus viem_core_view_set_number(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
/* Query row-selection margins/list labels in document-layout DIPs. Exact current
 * geometry is required. Excludes two DIPs before prose content; a table row's
 * margin reaches its table's left edge, where native row controls also appear.
 * Native number columns remain independent fixed hot areas. */
ViemStatus viem_core_view_row_selection_hit_test(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, float x, float y, uint32_t *out_hit);
/* Select a complete displayed row at document-layout y. Table gutter gestures
 * select the entire semantic table row. Extension retains the initial row. */
ViemStatus viem_core_view_select_pointer_row(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, float y, uint32_t extend_selection,
    ViemCoreOutcome *out_outcome);
/* One-based physical source lines independent of command line mode. Regions
 * intersect the viewport but retain their full wrapped line bounds. Entirely
 * hidden syntax has no independent glyph band; visible folded source lines
 * share their rendered band. ESTIMATED marks huge partially measured lines.
 * These queries perform no shaping or text materialization. */
ViemStatus viem_core_view_line_number_snapshot_info(ViemCoreHandle core, ViemViewId view,
                                                  ViemLineNumberSnapshotInfo *out_info);
ViemStatus viem_core_view_copy_line_number_regions(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, const ViemLayoutRect *expected_viewport,
    ViemLineNumberRegion *regions, uint64_t capacity, uint64_t *out_count);

#define VIEM_LINE_MODE_VISUAL 0u
#define VIEM_LINE_MODE_PHYSICAL_SOURCE 1u
/* View-local visual or physical source-line command policy. out_fixed is 1
 * when the format always uses that mode (Code uses physical source lines);
 * setting the other mode then fails with UNSUPPORTED_OPERATION. */
ViemStatus viem_core_view_line_mode(ViemCoreHandle core, ViemViewId view, uint32_t *out_mode, uint32_t *out_fixed);
/* Base Paragraph font size with line spacing and this view's zoom applied.
 * Does not perform layout or consult the visible text's styles/font metrics. */
ViemStatus viem_core_view_default_line_height(ViemCoreHandle core, ViemViewId view, float *out_height);
ViemStatus viem_core_view_default_column_width(ViemCoreHandle handle, ViemViewId view, float *out_width);
ViemStatus viem_core_view_set_line_mode(ViemCoreHandle core, ViemViewId view, uint32_t mode, ViemCoreOutcome *out_outcome);
/* Application input preference; enabled must be 0 or 1. No source/undo change. */
ViemStatus viem_core_view_set_smart_quotes(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
ViemStatus viem_core_view_set_markdown_autodetect(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
/* Home toggles between the first nonblank and the line start; enabled is 0 or 1. */
ViemStatus viem_core_view_set_smart_home(ViemCoreHandle core, ViemViewId view, uint32_t enabled);
ViemStatus viem_core_view_set_wrap(ViemCoreHandle core, ViemViewId view,
                                   uint32_t wrap,
                                   ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_set_file_format(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetFileFormat *request, ViemCoreOutcome *out_outcome);

ViemStatus viem_core_view_list_selection(
    ViemCoreHandle core, ViemViewId view,
    ViemLogicalSelectionIdentity *out_selection);
ViemStatus viem_core_view_table_context(ViemCoreHandle core, ViemViewId view,
    ViemTableContext *out_context);
ViemStatus viem_core_view_table_context_at(ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t document_revision, uint64_t text_offset,
    ViemTableContext *out_context);
ViemStatus viem_core_view_insert_table(ViemCoreHandle core, ViemViewId view,
    const ViemInsertTable *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_table_action(ViemCoreHandle core, ViemViewId view,
    const ViemTableAction *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_copy_table_cells(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, ViemTableCell *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_table_selection(ViemCoreHandle core, ViemViewId view,
    ViemTableSelection *out_selection);
ViemStatus viem_core_view_select_table_cells(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelection *request, ViemCoreOutcome *out_outcome);
/* Complete exact cell selection: no clipboard effects; row-major ranges include
 * offscreen/empty cells. Pointer regions must be pairwise disjoint. */
ViemStatus viem_core_view_copy_table_selection_text(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelection *expected, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_copy_table_selection_ranges(ViemCoreHandle core, ViemViewId view,
    const ViemTableSelection *expected, ViemFormattedUtf8Range *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_core_view_set_block_quote(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetBlockQuote *request, ViemCoreOutcome *out_outcome);
/* Make the exact selection an alert of request->kind, quoting unquoted blocks,
 * or turn its alerts back into plain quotes. Visible text never changes. */
ViemStatus viem_core_view_set_alert(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetAlert *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_set_list_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetListStyle *request, ViemCoreOutcome *out_outcome);
/* Exact current selection; capability bits reflect verified structural edits. */
ViemStatus viem_core_view_list_indent_capabilities(
    ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentity *expected_selection, uint32_t *out_flags);
/* Insert a paragraph in the same Markdown WYSIWYG list item. Requires an exact
 * collapsed eligible caret; Normal mode enters Insert. */
ViemStatus viem_core_view_insert_list_paragraph(
    ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentity *expected_selection, ViemCoreOutcome *out_outcome);
/* unindent is 0 or 1. A top-level item cannot be unindented. */
ViemStatus viem_core_view_indent_list(
    ViemCoreHandle core, ViemViewId view,
    const ViemListIndent *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_set_paragraph_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemSetParagraphStyle *request, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_assign_style(
    ViemCoreHandle handle, ViemViewId view,
    const ViemAssignStyle *request, ViemCoreOutcome *out_outcome);
/*
 * Begin one exact frontend-owned style gesture. Only one may be active in a
 * core. An empty group creates no history entry. Any ordinary coordinator
 * event or removal of the owner closes the successful prefix and consumes the
 * capability before continuing.
 */
ViemStatus viem_core_view_begin_style_edit_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleSheetIdentity *expected,
    ViemStyleEditGroup *out_group);
/* One exact editable-definition field edit through its source/configuration
 * authority, with a standalone undo unit. */
ViemStatus viem_core_view_edit_style(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEdit *request, ViemCoreOutcome *out_outcome);
/*
 * Commit one immediately visible exact edit into group. request must carry the
 * current style-sheet identity on every call. A failed/no-op edit leaves the
 * group usable and does not poison previously successful edits.
 */
ViemStatus viem_core_view_edit_style_in_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEditGroup *group,
    const ViemStyleEdit *request, ViemCoreOutcome *out_outcome);
/*
 * Consume group and close its successful edits as one undo/redo unit. This
 * never rolls back. Reuse after end returns INVALID_STYLE_EDIT_GROUP.
 */
ViemStatus viem_core_view_end_style_edit_group(
    ViemCoreHandle core, ViemViewId view,
    const ViemStyleEditGroup *group);

/* Set/read Markdown underline, superscript or subscript. */
ViemStatus viem_core_view_set_inline_property(
    ViemCoreHandle handle, ViemViewId view,
    const ViemLogicalSelectionIdentity *expected_selection,
    uint32_t property, uint8_t enabled, ViemCoreOutcome *out_outcome);
ViemStatus viem_core_view_inline_property_state(
    ViemCoreHandle handle, ViemViewId view, uint32_t property, uint32_t *out_state);

/* Set Markdown strikethrough at an exact selection or typing caret. */
ViemStatus viem_core_view_set_strikethrough(ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentity *expected_selection, uint8_t enabled,
    ViemCoreOutcome *out_outcome);
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
 * or invokes its measurement provider. Markdown exports its formatted structure;
 * Code analyzes the complete captured snapshot, including offscreen text.
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
/* Structural table membership, for layout; formatting availability and state
 * come from viem_core_view_formatting_actions. */
#define VIEM_SELECTED_STYLE_HAS_TABLE (1u << 2)
typedef struct ViemSelectedStylesInfo {
  uint32_t flags;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t style_sheet_revision;
  uint64_t paragraph_id_bytes;
  uint64_t character_id_bytes;
} ViemSelectedStylesInfo;
ViemStatus viem_core_view_selected_styles_export(ViemCoreHandle core,
    ViemViewId view, uint64_t expected_revision,
    ViemSelectedStylesInfo *out_info, uint8_t *out_utf8, uint64_t capacity);

/* Native formatting actions offered by toolbars, menus and accelerators.
 * Core decides each action's availability and on/off/mixed state for the
 * view's selection once for every frontend. Each mask uses bit 1 << action.
 * Alerts are ALERT_NOTE + kind - VIEM_ALERT_NOTE; HEADING_0 + level names a
 * heading level, with level 0 as Base Paragraph. */
#define VIEM_FORMATTING_BOLD 1u
#define VIEM_FORMATTING_ITALIC 2u
#define VIEM_FORMATTING_UNDERLINE 3u
#define VIEM_FORMATTING_STRIKETHROUGH 4u
#define VIEM_FORMATTING_SUPERSCRIPT 5u
#define VIEM_FORMATTING_SUBSCRIPT 6u
#define VIEM_FORMATTING_CODE 7u
#define VIEM_FORMATTING_CODE_BLOCK 8u
#define VIEM_FORMATTING_BLOCK_QUOTE 9u
#define VIEM_FORMATTING_ALERT_NOTE 10u
#define VIEM_FORMATTING_ALERT_TIP 11u
#define VIEM_FORMATTING_ALERT_IMPORTANT 12u
#define VIEM_FORMATTING_ALERT_WARNING 13u
#define VIEM_FORMATTING_ALERT_CAUTION 14u
#define VIEM_FORMATTING_BULLETED_LIST 15u
#define VIEM_FORMATTING_NUMBERED_LIST 16u
#define VIEM_FORMATTING_TASK_LIST 17u
#define VIEM_FORMATTING_REMOVE_LIST 18u
#define VIEM_FORMATTING_INDENT 19u
#define VIEM_FORMATTING_UNINDENT 20u
#define VIEM_FORMATTING_LIST_PARAGRAPH 21u
#define VIEM_FORMATTING_HEADING_0 22u
#define VIEM_FORMATTING_HEADING_1 23u
#define VIEM_FORMATTING_HEADING_2 24u
#define VIEM_FORMATTING_HEADING_3 25u
#define VIEM_FORMATTING_HEADING_4 26u
#define VIEM_FORMATTING_HEADING_5 27u
#define VIEM_FORMATTING_HEADING_6 28u
#define VIEM_FORMATTING_LINK 29u
#define VIEM_FORMATTING_REFERENCE 30u
#define VIEM_FORMATTING_IMAGE 31u
#define VIEM_FORMATTING_TABLE 32u
#define VIEM_FORMATTING_PARAGRAPH_STYLES 33u
#define VIEM_FORMATTING_CHARACTER_STYLES 34u
typedef struct ViemFormattingActions {
  uint64_t enabled;
  uint64_t on;
  uint64_t mixed;
} ViemFormattingActions;
/* Only actions in `requested` are evaluated, so validating one menu item stays
 * cheap; the rest report disabled and off. Unknown bits are ignored, so
 * UINT64_MAX requests every action. */
ViemStatus viem_core_view_formatting_actions(ViemCoreHandle core, ViemViewId view,
    uint64_t requested, ViemFormattingActions *out_actions);

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
ViemStatus viem_theme_style_defaults_json(uint32_t variant, const uint8_t *json, uint64_t length, uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_theme_validate_json(const uint8_t *json, uint64_t length);
ViemStatus viem_theme_validate_name(const uint8_t *name, uint64_t length);
/* Import a Vim color scheme (bounded to 4 MiB) as complete theme JSON based
 * on the built-in Paper or Midnight preset, using the two-pass output contract.
 * Evaluation is deterministic and touches no files. On success the callback
 * receives skipped statements once the output is written; on failure, the
 * reason. Unique names derive from a file stem: filename-safe, at most 32
 * characters, distinct ignoring case from the LF-separated existing names. */
ViemStatus viem_theme_import_vim(const uint8_t *source, uint64_t length, uint8_t *output, uint64_t capacity, uint64_t *required, ViemStyleDefaultsDiagnosticCallback diagnostic, void *context);
ViemStatus viem_theme_unique_name(const uint8_t *stem, uint64_t stem_length, const uint8_t *existing, uint64_t existing_length, uint8_t *output, uint64_t capacity, uint64_t *required);

/* Command prompt selection and editing carry the exact exported prompt identity. */
typedef struct ViemCommandLineSelection {
  uint64_t anchor_utf8_offset;
  uint64_t active_utf8_offset;
} ViemCommandLineSelection;
ViemStatus viem_core_view_command_line_selection(ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentity *expected, ViemCommandLineSelection *out_selection);
ViemStatus viem_core_view_edit_command_line(ViemCoreHandle core, ViemViewId view,
    const ViemCommandLineIdentity *expected, uint32_t operation,
    uint64_t start, uint64_t end, const uint8_t *utf8, uint64_t length,
    ViemCoreOutcome *out_outcome);

ViemStatus viem_core_view_set_encoding_with_effects(ViemCoreHandle core, ViemViewId view, const ViemSetEncoding *request, ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effects);
/* Copy exact half-open source-byte bounds from the named source revision. */
ViemStatus viem_core_copy_source_range_bytes(ViemCoreHandle core, uint64_t document, uint64_t revision, uint64_t source_start, uint64_t source_end, uint8_t *output, uint64_t capacity, uint64_t *out_required, uint32_t *out_complete);

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
    ViemCoreHandle handle, const ViemFormattedUtf8Range *request,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_effect_batch_copy_clipboard_json(
    ViemEffectBatchHandle batch, uint64_t clipboard_index,
    uint8_t *output, uint64_t output_capacity, uint64_t *out_required);

/* Application-wide Code stylesheet. Its identity has document/revision zero.
 * Changes never create a buffer transaction. UTF-8/typed output arrays use
 * the same exact-capacity and disjoint-pointer rules as document styles. */
/* Exact-snapshot local parent candidates as a UTF-8 JSON array of stable IDs.
 * Null/zero output queries required bytes. All input/output regions are disjoint. */
/* Pure shared config.json validation/defaults. Returns UTF-8 JSON with
 * configuration and error fields, one null. Unknown keys survive. No file I/O.
 * Input/output/required must be disjoint. Size query uses NULL, 0 output. */
ViemStatus viem_application_config(const uint8_t *input, uint64_t input_length,
    uint8_t *output, uint64_t capacity, uint64_t *required);

ViemStatus viem_core_style_parent_candidates(ViemCoreHandle core,
    const ViemStyleSheetIdentity *expected, uint32_t namespace_id, ViemUtf8Slice child,
    uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_code_style_sheet_info(ViemStyleSheetInfo *output);
ViemStatus viem_code_copy_style_sheet(const ViemStyleSheetIdentity *expected,
    ViemStyleDefinition *definitions, uint64_t definition_capacity,
    ViemStyleProperty *properties, uint64_t property_capacity,
    ViemStyleValueItem *value_items, uint64_t value_item_capacity,
    ViemStyleDependency *dependencies, uint64_t dependency_capacity,
    uint8_t *string_bytes, uint64_t string_capacity, ViemStyleSheetInfo *out_info);
ViemStatus viem_code_edit_style(const ViemStyleEdit *request, ViemStyleSheetInfo *output);
ViemStatus viem_code_create_style(const ViemCreateStyle *request, ViemStyleSheetInfo *output);
ViemStatus viem_code_delete_style(const ViemDeleteStyle *request, ViemStyleSheetInfo *output);
/* Generates the implicit definition for one syntax name, and its missing
 * dotted ancestors, when it has no definition. Fails with ResourceExhausted
 * when the implicit-definition limit leaves the name undefined. */
ViemStatus viem_code_materialize_style(const uint8_t *name, uint64_t length,
    ViemStyleSheetInfo *output);
ViemStatus viem_code_replace_style_json(const uint8_t *input, uint64_t length);
ViemStatus viem_code_export_style_json(uint8_t *output, uint64_t capacity, uint64_t *required);
/* Buffer-local source-preserving mode override. Auto returns to detection. */
#define VIEM_DOCUMENT_MODE_AUTO 0u
#define VIEM_DOCUMENT_MODE_PLAIN_TEXT 1u
#define VIEM_DOCUMENT_MODE_MARKDOWN 2u
#define VIEM_DOCUMENT_MODE_CODE 3u
typedef struct ViemSetDocumentMode {
    uint32_t mode;
    uint64_t document_id;
    uint64_t document_revision;
} ViemSetDocumentMode;
/* Code with an empty language retains the current language selection and always
   selects Code format. Only Code accepts a nonempty language ID, which selects
   that language explicitly. All input/output regions must be disjoint. */
ViemStatus viem_core_view_set_document_mode_with_effects(ViemCoreHandle core, ViemViewId view,
    const ViemSetDocumentMode *request, const uint8_t *language, uint64_t length,
    ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effects);
/* Two-pass UTF-8 JSON queries. Output regions must be disjoint. Mode reads
   cached detection; language enumeration does not load or execute providers. */
ViemStatus viem_core_copy_document_mode_json(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_copy_code_languages_json(uint8_t *output, uint64_t capacity, uint64_t *required);
ViemStatus viem_core_initialize_code_detection(ViemCoreHandle core, const uint8_t *filename, uint64_t length, uint8_t allow_auto_code);
/* Instead of the call above, for a core created in a reloaded buffer's format
   to replace it: keep the previous core's language choice, and turn Text it
   didn't choose by hand into Code when the new source declares a language. */
ViemStatus viem_core_initialize_reloaded_code_detection(ViemCoreHandle core, const uint8_t *filename, uint64_t length, ViemCoreHandle previous);
ViemStatus viem_core_configure_syntax(ViemCoreHandle core, const uint8_t *vim_directory, uint64_t length);
ViemStatus viem_core_set_code_filename_associations_json(ViemCoreHandle core, const uint8_t *json, uint64_t length);
/* selection: Automatic=0, None=1, Language=2. Only Language takes a nonempty name. */
ViemStatus viem_core_set_code_language(ViemCoreHandle core, uint32_t selection, const uint8_t *language, uint64_t length);
ViemStatus viem_core_redetect_code_language(ViemCoreHandle core, const uint8_t *filename, uint64_t length);
/* Link popup JSON: selection {viewId,documentId,revision,start,end,kind,anchor,
 * active,affinity}, canInsert, linked, canExitLink, canRemoveSelection, text,
 * link null or {start,end,text,destination,
 * editable}. Query work is bounded to the active source region. */
ViemStatus viem_core_view_copy_link_context(ViemCoreHandle core, ViemViewId view,
                                           uint8_t *output, uint64_t capacity,
                                           uint64_t *required);
/* action: 0 inserts at expected selection, 1 edits, 2 removes active link,
 * 3 exits link treatment for future typing, 4 removes selected link treatment.
 * Every mutation validates the complete expected logical selection identity. */
ViemStatus viem_core_view_edit_link(ViemCoreHandle core, ViemViewId view,
                                   const ViemLogicalSelectionIdentity *expected,
                                   uint32_t action, uint64_t link_start,
                                   uint64_t link_end, ViemUtf8Slice text,
                                   ViemUtf8Slice destination,
                                   ViemCoreOutcome *outcome);
/* Reference actions use bounded UTF-8 JSON with action and offset; edits also
 * carry their named fields. Validates the complete expected selection identity.
 * Link context additionally exports nullable reference settings and uses. */
ViemStatus viem_core_view_reference_action(ViemCoreHandle core, ViemViewId view,
    const ViemLogicalSelectionIdentity *expected, ViemUtf8Slice request,
    ViemCoreOutcome *outcome);
/* Image popup has the link-context schema with "image" instead of "link".
 * The range is one atomic U+FFFC object in Markdown Preview.
 * Image entries also contain nullable width/height pixel sizes.
 * No query fetches resources. Empty alternative text is allowed. */
ViemStatus viem_core_view_copy_image_context(ViemCoreHandle core, ViemViewId view,
                                            uint8_t *output, uint64_t capacity,
                                            uint64_t *required);
/* action: 0 inserts, 1 edits, 2 removes the whole image.
 * Width/height are positive pixel sizes; zero clears that explicit dimension. */
ViemStatus viem_core_view_edit_image(ViemCoreHandle core, ViemViewId view,
                                    const ViemLogicalSelectionIdentity *expected,
                                    uint32_t action, uint64_t image_start,
                                    uint64_t image_end, ViemUtf8Slice text,
                                    ViemUtf8Slice destination,
                                    uint32_t width, uint32_t height,
                                    ViemCoreOutcome *outcome);
/* Select the complete WYSIWYG image independently of pointer preferences. */
ViemStatus viem_core_view_select_image(ViemCoreHandle core, ViemViewId view,
                                      uint64_t document_id, uint64_t revision,
                                      uint64_t text_offset, ViemCoreOutcome *outcome);
/* Explicit heading picker: {headings:[{text,destination,level,offset}],truncated}.
 * Exact revision, two-pass UTF-8 JSON. Enumerated only on user request. */
ViemStatus viem_core_copy_reference_choices(ViemCoreHandle core, uint64_t document_id,
                                       uint64_t revision, uint8_t *output,
                                       uint64_t capacity, uint64_t *required);
ViemStatus viem_core_copy_link_headings(ViemCoreHandle core, uint64_t document_id,
                                       uint64_t revision, uint8_t *output,
                                       uint64_t capacity, uint64_t *required);
/* Empty language selects None. The offset must name the original code block
 * in this exact document revision; stale popup actions never retarget. */
ViemStatus viem_core_view_set_code_block_language(
    ViemCoreHandle core, ViemViewId view, uint64_t document_id,
    uint64_t document_revision, uint64_t offset, ViemUtf8Slice language,
    ViemCoreOutcome *output);

/* Explicit navigation in this exact document revision. fragment is percent-
 * decoded and excludes '#'. L<number> targets a one-based physical source line,
 * centered within valid scroll limits; other fragments target Markdown headings.
 * Missing targets set found=0 and leave the view unchanged. */
ViemStatus viem_core_view_navigate_link_fragment(
    ViemCoreHandle core, ViemViewId view, uint64_t document_id, uint64_t revision,
    ViemUtf8Slice fragment, uint8_t *found, ViemCoreOutcome *outcome);

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
/* View-local presentation only. Off by default; applies in Code with a
 * proportionally spaced resolved font. Source and undo are unchanged. */
ViemStatus viem_core_view_set_align_indents_to_previous_line(ViemCoreHandle core, ViemViewId view, uint8_t enabled);
/* Exact presentation-snapshot and viewport UTF-8 JSON batch:
   {style, markers:[{text,rowIndex,x,y,width,height}], enabled, applicable}.
   All input/output regions must be disjoint. Count and copy pass the same
   finite viewport (nonnegative origin and dimensions). A changed viewport
   returns VIEM_STATUS_STALE_REVISION even when cached layout is reused. */
ViemStatus viem_core_view_copy_whitespace_markers(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, const ViemLayoutRect *expected_viewport,
    uint8_t *output, uint64_t capacity, uint64_t *out_length);
ViemStatus viem_core_poll_syntax(ViemCoreHandle core, uint8_t *changed);
/* Wait up to the shared 100 ms syntax grace period, without holding the core.
 * Materialize the viewport first; refresh invalidated layout when changed.
 * Repeated presentation of the same timed-out request does not wait again. */
ViemStatus viem_core_view_wait_for_syntax(ViemCoreHandle core, ViemViewId view, uint8_t *changed);
ViemStatus viem_core_copy_syntax_diagnostics(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);
/* Read up to 8 KiB of distinct warnings for this exact immutable layout.
 * Standard two-pass UTF-8 output; stale snapshots are rejected, never replaced. */
ViemStatus viem_core_view_copy_layout_diagnostics(ViemCoreHandle core, ViemViewId view, const ViemLayoutSnapshotIdentity *expected, uint8_t *output, uint64_t capacity, uint64_t *required);
/* Read-only two-pass UTF-8 JSON array of unique sorted names in accepted syntax
   runs, including undefined names. Does not parse, publish, or scan source. */
ViemStatus viem_core_copy_syntax_style_names(ViemCoreHandle core, uint8_t *output, uint64_t capacity, uint64_t *required);

/* Host-loaded `:read` continuation. */
ViemStatus viem_core_view_read_file(ViemCoreHandle core, ViemViewId view,
    uint64_t document_id, uint64_t revision, uint32_t source, uint64_t after_offset,
    const uint8_t *bytes, uint64_t length, ViemCoreOutcome *out_outcome);
/* Run one command line as if typed after `:` (Vim script: `|`-separated
   commands, :let, :call, :source). The view must be in Normal mode. */
ViemStatus viem_core_view_execute_command_line(ViemCoreHandle core, ViemViewId view,
    const uint8_t *text, uint64_t length,
    const ViemCommandTurnContext *context, ViemCoreOutcome *out_outcome,
    ViemEffectBatchHandle *out_effects);
/* Fire a native lifecycle event. Call BufWritePre before capturing save
   bytes and BufWritePost only after a successful write. Empty match uses
   the buffer filename. Returns ordinary command effects. */
ViemStatus viem_core_view_fire_autocmd(ViemCoreHandle core, ViemViewId view,
    const uint8_t *event, uint64_t event_length,
    const uint8_t *matched, uint64_t matched_length,
    const ViemCommandTurnContext *context, ViemCoreOutcome *out_outcome,
    ViemEffectBatchHandle *out_effects);
/* Resume after all preceding native requests succeed, or cancel on failure.
   Pass the original core/view and returned token. Stale tokens are rejected. */
ViemStatus viem_core_view_resume_script(ViemCoreHandle core, ViemViewId view,
    uint64_t token, uint32_t success, const ViemCommandTurnContext *context,
    ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effects);
/* Drain startup application effects once the first native view exists. */
ViemStatus viem_core_view_take_startup_effects(ViemCoreHandle core, ViemViewId view,
    ViemCoreOutcome *out_outcome, ViemEffectBatchHandle *out_effects);
/* Replace the theme names `:colorscheme` matches, ignoring case: UTF-8
   names separated by NUL bytes. Call at startup and when themes change. */
ViemStatus viem_set_theme_names(const uint8_t *names, uint64_t length);

/* The paragraph must have a checkbox in this exact current layout snapshot. */
ViemStatus viem_core_view_toggle_checkbox(ViemCoreHandle core, ViemViewId view,
    const ViemLayoutSnapshotIdentity *expected, uint64_t paragraph_id,
    ViemCoreOutcome *out_outcome);

/* Diff mode (docs/diff_mode.md). A session compares a read-only base core
 * with a live core and installs each side's presentation in a view. Base
 * loads and file lists may block on disk or a `git` process: call them on a
 * worker. They return an owned result even when the base is unavailable;
 * only invalid arguments fail the call. Release every result. */
typedef uint64_t ViemDiffSessionHandle;
typedef uint64_t ViemDiffResultHandle;
typedef uint64_t ViemDiffCancellationHandle;

#define VIEM_DIFF_SESSION_ALIGNED (1u << 0)
/* Unchanged runs collapse behind a divider (see docs/diff_mode.md). */
#define VIEM_DIFF_SESSION_COLLAPSED (1u << 1)

#define VIEM_DIFF_SYNC_UNCHANGED 0u
#define VIEM_DIFF_SYNC_INCREMENTAL 1u
#define VIEM_DIFF_SYNC_FULL 2u

#define VIEM_DIFF_NAVIGATE_NEXT 0u
#define VIEM_DIFF_NAVIGATE_PREVIOUS 1u
/* A change was found; otherwise the sides are identical and nothing moved. */
#define VIEM_DIFF_NAVIGATION_FOUND (1u << 0)
/* The views were repositioned, not only the caret. */
#define VIEM_DIFF_NAVIGATION_SCROLLED (1u << 1)

/* The mark is the new side's added lines; otherwise removed lines. */
#define VIEM_DIFF_MARK_ADDED (1u << 0)

/* LAST_SAVED loads the file on disk (Refresh); the in-memory copy comes from
 * viem_core_copy_saved_source. FILE loads the chosen file_path. */
#define VIEM_DIFF_BASE_LAST_SAVED 0u
#define VIEM_DIFF_BASE_UNSTAGED 1u
#define VIEM_DIFF_BASE_HEAD 2u
#define VIEM_DIFF_BASE_BRANCH 3u
#define VIEM_DIFF_BASE_FILE 4u
#define VIEM_DIFF_BASE_COMMIT 5u
#define VIEM_DIFF_FILE_PICKER_LIMIT 256u

#define VIEM_DIFF_RESULT_OK 0u
#define VIEM_DIFF_RESULT_UNAVAILABLE 1u
#define VIEM_DIFF_RESULT_CANCELLED 2u
#define VIEM_DIFF_RESULT_TIMED_OUT 3u
#define VIEM_DIFF_RESULT_TOO_LARGE 4u
#define VIEM_DIFF_RESULT_IO_ERROR 5u
#define VIEM_DIFF_RESULT_COMMAND_FAILED 6u
/* The base exists in that version; otherwise its bytes are empty. */
#define VIEM_DIFF_RESULT_BASE_EXISTS (1u << 0)

#define VIEM_DIFF_FILE_MODIFIED 0u
#define VIEM_DIFF_FILE_ADDED 1u
#define VIEM_DIFF_FILE_DELETED 2u
#define VIEM_DIFF_FILE_RENAMED 3u
#define VIEM_DIFF_FILE_UNSAVED 4u

typedef struct ViemDiffBaseRequest {
  uint32_t kind;
  /* Absolute UTF-8 path of the document, the chosen base file, or a renamed
   * file's base path. */
  ViemUtf8Slice file_path;
  /* Branch name or full commit hash. A commit query also accepts a prefix. */
  ViemUtf8Slice revision;
  /* Empty means `git` found through PATH. */
  ViemUtf8Slice git_program;
  /* Zero for none. */
  ViemDiffCancellationHandle cancellation;
  /* File lists only: open buffers with unsaved changes join the list. */
  const ViemUtf8Slice *unsaved_paths;
  uint64_t unsaved_path_count;
  /* File lists only: an absolute folder whose files alone are listed, as
   * `git diff -- <folder>` does. Empty lists the whole repository. */
  ViemUtf8Slice scope_path;
} ViemDiffBaseRequest;

typedef struct ViemDiffResultInfo {
  uint32_t flags;
  /* A VIEM_DIFF_RESULT_* code. */
  uint32_t result;
  uint64_t byte_count;
  uint64_t item_count;
  uint64_t message_bytes;
} ViemDiffResultInfo;

/* A list item's path (or branch name) bytes, then its base path bytes, are
 * copied back to back into the caller's text buffer. */
typedef struct ViemDiffListItem {
  uint32_t change;
  uint64_t path_bytes;
  uint64_t base_path_bytes;
  uint64_t added;
  uint64_t removed;
} ViemDiffListItem;

typedef struct ViemDiffSessionInfo {
  uint32_t flags;
  uint64_t hunk_count;
  uint64_t added;
  uint64_t removed;
  uint64_t base_line_count;
  uint64_t new_line_count;
} ViemDiffSessionInfo;

typedef struct ViemDiffNavigation {
  /* VIEM_DIFF_NAVIGATION_* flags. */
  uint32_t flags;
  uint64_t change_index;
  uint64_t change_count;
  /* The new side's zero-based hard line that received the caret. */
  uint64_t caret_line;
} ViemDiffNavigation;

/* One stretch of a scrollbar track, in layout units from the document top. */
typedef struct ViemDiffScrollbarMark {
  float top;
  float bottom;
  /* VIEM_DIFF_MARK_* flags. */
  uint32_t flags;
} ViemDiffScrollbarMark;

ViemStatus viem_diff_session_create(ViemCoreHandle base_core, ViemCoreHandle new_core,
    uint32_t flags, ViemDiffSessionHandle *out_session);
ViemStatus viem_diff_session_destroy(ViemDiffSessionHandle session);
ViemStatus viem_diff_session_set_aligned(ViemDiffSessionHandle session, uint32_t aligned);
/* Collapse unchanged runs, or show them all; either way every run the user
 * opened closes again. The next sync applies it. */
ViemStatus viem_diff_session_set_collapsed(ViemDiffSessionHandle session, uint32_t collapsed);
/* Open the collapsed run whose divider borders the zero-based hard `line` on
 * one side, as a click on a VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT
 * decoration of that line's row does: the run after it, or a run at the start
 * of the file before it. out_opened is 1 when a run opened; sync next to show
 * it on both sides. */
ViemStatus viem_diff_session_open_collapsed(ViemDiffSessionHandle session,
    uint32_t from_new, uint64_t line, uint32_t *out_opened);
/* Update the diff after any change to either core and install both sides'
 * presentations; out_sync is a VIEM_DIFF_SYNC_* value. */
ViemStatus viem_diff_session_sync(ViemDiffSessionHandle session,
    ViemCoreHandle base_core, ViemViewId base_view,
    ViemCoreHandle new_core, ViemViewId new_view, uint32_t *out_sync);
/* The zero-based hard line on the other side corresponding to `line`. */
ViemStatus viem_diff_session_corresponding_line(ViemDiffSessionHandle session,
    uint32_t from_new, uint64_t line, uint64_t *out_line);
/* Refresh the shared vertical range and mirror the leading pane's origin. */
#define VIEM_DIFF_SCROLL_BASE_CHANGED 1u
#define VIEM_DIFF_SCROLL_NEW_CHANGED 2u
ViemStatus viem_diff_session_sync_scroll(ViemDiffSessionHandle session,
    ViemCoreHandle base_core, ViemViewId base_view, ViemCoreHandle new_core,
    ViemViewId new_view, uint32_t from_new, uint32_t *out_changed);
ViemStatus viem_diff_session_info(ViemDiffSessionHandle session,
    ViemDiffSessionInfo *out_info);
/* Move both sides to the next or previous change (a VIEM_DIFF_NAVIGATE_*
 * direction), wrapping within the file: the new view's caret moves to the
 * change, and both views scroll unless it is already visible on both. Then
 * refresh both views. */
ViemStatus viem_diff_session_navigate(ViemDiffSessionHandle session,
    ViemCoreHandle base_core, ViemViewId base_view,
    ViemCoreHandle new_core, ViemViewId new_view, uint32_t direction,
    ViemDiffNavigation *out_navigation);
/* The backgrounds of the Added line and Removed line Code styles, for
 * scrollbar marks; the built-in defaults when a style declares none. */
ViemStatus viem_diff_line_colors(ViemRgba *out_added, ViemRgba *out_removed);
/* Two-pass copy of one side's scrollbar marks, sorted by top, and the
 * document extent they are measured against; is_new is one for the live
 * side. A short buffer reports the count with VIEM_STATUS_BUFFER_TOO_SMALL. */
ViemStatus viem_diff_session_scrollbar_marks(ViemDiffSessionHandle session,
    ViemCoreHandle core, ViemViewId view, uint32_t is_new,
    ViemDiffScrollbarMark *marks, uint64_t capacity, uint64_t *out_count,
    float *out_extent);
/* Set the buffer-local `modifiable` policy; zero rejects every edit path. */
ViemStatus viem_core_set_modifiable(ViemCoreHandle core, uint32_t modifiable);
/* Remove diff presentation from a view when leaving diff mode. */
ViemStatus viem_diff_clear_view(ViemCoreHandle core, ViemViewId view);
/* out_opened is 1 when the view's caret moved onto collapsed context, which
 * opened in that view, since the last sync: sync now to open it on the other
 * side too. */
ViemStatus viem_diff_view_opened_context(ViemCoreHandle core, ViemViewId view, uint32_t *out_opened);
/* Two-pass copy of the source as last loaded or saved (the Last saved base).
 * out_available is zero when it is unknown, such as after recovery. */
ViemStatus viem_core_copy_saved_source(ViemCoreHandle core, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required, uint32_t *out_available);

ViemStatus viem_diff_cancellation_create(ViemDiffCancellationHandle *out_cancellation);
/* Cancel loads using this token from any thread. */
ViemStatus viem_diff_cancellation_cancel(ViemDiffCancellationHandle cancellation);
ViemStatus viem_diff_cancellation_release(ViemDiffCancellationHandle cancellation);

ViemStatus viem_diff_load_base(const ViemDiffBaseRequest *request,
    ViemDiffResultHandle *out_result);
/* Git bases list tracked changes as `git diff` does plus unsaved buffers in
 * the repository; Last saved lists only the document; FILE lists nothing. */
ViemStatus viem_diff_list_files(const ViemDiffBaseRequest *request,
    ViemDiffResultHandle *out_result);
/* Returns UTF-8 JSON via result_copy_bytes: [{hash, short, title}].
 * HEAD lists up to 50 commits. COMMIT resolves one hexadecimal hash. */
ViemStatus viem_diff_query_commits(const ViemDiffBaseRequest *request,
                                  ViemDiffResultHandle *out_result);
ViemStatus viem_diff_list_branches(const ViemDiffBaseRequest *request,
    ViemDiffResultHandle *out_result);
ViemStatus viem_diff_result_info(ViemDiffResultHandle result, ViemDiffResultInfo *out_info);
ViemStatus viem_diff_result_copy_bytes(ViemDiffResultHandle result, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_diff_result_copy_message(ViemDiffResultHandle result, uint8_t *output,
    uint64_t output_capacity, uint64_t *out_required);
ViemStatus viem_diff_result_copy_item(ViemDiffResultHandle result, uint64_t index,
    ViemDiffListItem *out_item, uint8_t *text, uint64_t text_capacity,
    uint64_t *out_required);
ViemStatus viem_diff_result_release(ViemDiffResultHandle result);

#ifdef __cplusplus
}
#endif

#endif /* VIEM_CORE_H */
