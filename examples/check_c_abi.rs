//! Explicit native C ABI validation; see docs/abi-validation.md.

use std::path::Path;
use std::process::{Command, ExitCode};
use viem_core::ffi::*;

include!(concat!(env!("OUT_DIR"), "/abi_layout_checks.rs"));

fn main() -> ExitCode {
    match check_c_abi() {
        Ok(()) => {
            println!("C header matches Rust: {} struct layouts, {} field offsets, plus signature and constant assertions.", GENERATED_LAYOUT_COUNTS.0, GENERATED_LAYOUT_COUNTS.1);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("C ABI validation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check_c_abi() -> Result<(), String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_path = std::env::temp_dir().join(format!("viem-abi-{}.c", std::process::id()));
    std::fs::write(&source_path, probe_source())
        .map_err(|error| format!("could not write {}: {error}", source_path.display()))?;

    let mut compiler = if cfg!(target_env = "msvc") {
        let mut command = Command::new("cl");
        command.args(["/nologo", "/std:c11", "/WX", "/Zs", "/I"]);
        command
    } else {
        let mut command = Command::new("cc");
        command.args(["-std=c11", "-Werror", "-fsyntax-only", "-I"]);
        command
    };
    compiler.arg(manifest.join("include")).arg(&source_path);
    let result = compiler.output();
    // Clean up even when the compiler cannot be started or rejects the header.
    let _ = std::fs::remove_file(&source_path);
    let output = result.map_err(|error| {
        let setup = if cfg!(target_env = "msvc") {
            "Run from Visual Studio Developer PowerShell with the architecture matching Rust."
        } else {
            "Install the platform C compiler and ensure cc is on PATH."
        };
        format!("could not run {compiler:?}: {error}\n{setup}")
    })?;
    if !output.status.success() {
        return Err(format!(
            "{compiler:?} exited with {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    Ok(())
}

fn probe_source() -> String {
    let mut source = format!(
        r#"
#include "viem_core.h"
#include <stddef.h>
_Static_assert(VIEM_CORE_ABI_VERSION == {abi}, "ABI version");
_Static_assert(VIEM_LAYOUT_DECORATION_LANGUAGE_PICKER_DISABLED == {language_picker_disabled}, "fixed code language decoration flag");
_Static_assert(VIEM_LAYOUT_DECORATION_CHECKBOX == {checkbox_decoration}, "checkbox decoration flag");
_Static_assert(VIEM_LAYOUT_DECORATION_CHECKED == {checked_decoration}, "checked decoration flag");
_Static_assert(VIEM_LIST_STYLE_TASK == {task_style}, "task list style");
_Static_assert(VIEM_SELECTED_STYLE_HAS_TABLE == {has_table}, "table membership");
static ViemStatus (*toggle_checkbox)(ViemCoreHandle, ViemViewId, const ViemLayoutSnapshotIdentity *, uint64_t, ViemCoreOutcome *) = viem_core_view_toggle_checkbox;
_Static_assert(VIEM_LAYOUT_DECORATION_ALERT_ICON == {alert_icon}, "alert icon decoration flag");
_Static_assert(VIEM_LAYOUT_DECORATION_ALERT_TITLE == {alert_title}, "alert title decoration flag");
_Static_assert(VIEM_LAYOUT_DECORATION_ALERT_KIND_SHIFT == {alert_kind_shift}, "alert decoration kind shift");
_Static_assert(VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT == {collapsed_context}, "collapsed context decoration flag");
_Static_assert(VIEM_ALERT_CAUTION == {alert_caution}, "last alert kind");
static ViemStatus (*set_alert)(ViemCoreHandle, ViemViewId, const ViemSetAlert *, ViemCoreOutcome *) = viem_core_view_set_alert;
_Static_assert(VIEM_FORMATTING_BOLD == {formatting_bold}, "first formatting action");
_Static_assert(VIEM_FORMATTING_ALERT_NOTE == {formatting_alert_note}, "first alert formatting action");
_Static_assert(VIEM_FORMATTING_HEADING_0 == {formatting_heading_0}, "base paragraph formatting action");
_Static_assert(VIEM_FORMATTING_CHARACTER_STYLES == {formatting_character_styles}, "last formatting action");
static ViemStatus (*formatting_actions)(ViemCoreHandle, ViemViewId, uint64_t, ViemFormattingActions *) = viem_core_view_formatting_actions;
static ViemStatus (*set_smart_home)(ViemCoreHandle, ViemViewId, uint32_t) = viem_core_view_set_smart_home;
_Static_assert(VIEM_KEY_ALT_ENTER == {alt_enter}, "list paragraph key");
_Static_assert(VIEM_LIST_CAN_INSERT_PARAGRAPH == {list_paragraph_capability}, "list paragraph capability");
static ViemStatus (*insert_list_paragraph)(ViemCoreHandle, ViemViewId, const ViemLogicalSelectionIdentity *, ViemCoreOutcome *) = viem_core_view_insert_list_paragraph;
_Static_assert(sizeof(ViemSetDocumentMode) == {set_mode_size}, "document mode operation size");
static ViemStatus (*read_file_bytes)(ViemCoreHandle, ViemViewId, uint64_t, uint64_t, uint32_t, uint64_t, const uint8_t *, uint64_t, ViemCoreOutcome *) = viem_core_view_read_file;
static ViemStatus (*execute_command_line)(ViemCoreHandle, ViemViewId, const uint8_t *, uint64_t, const ViemCommandTurnContext *, ViemCoreOutcome *, ViemEffectBatchHandle *) = viem_core_view_execute_command_line;
static ViemStatus (*resume_script)(ViemCoreHandle, ViemViewId, uint64_t, uint32_t, const ViemCommandTurnContext *, ViemCoreOutcome *, ViemEffectBatchHandle *) = viem_core_view_resume_script;
static ViemStatus (*take_startup_effects)(ViemCoreHandle, ViemViewId, ViemCoreOutcome *, ViemEffectBatchHandle *) = viem_core_view_take_startup_effects;
static ViemStatus (*fire_autocmd)(ViemCoreHandle, ViemViewId, const uint8_t *, uint64_t, const uint8_t *, uint64_t, const ViemCommandTurnContext *, ViemCoreOutcome *, ViemEffectBatchHandle *) = viem_core_view_fire_autocmd;
static ViemStatus (*set_theme_names)(const uint8_t *, uint64_t) = viem_set_theme_names;
static ViemStatus (*copy_source_bytes)(ViemCoreHandle, uint64_t, uint64_t, uint64_t, uint64_t, uint8_t *, uint64_t, uint64_t *, uint32_t *) = viem_core_copy_source_range_bytes;
static ViemStatus (*wait_for_syntax)(ViemCoreHandle, ViemViewId, uint8_t *) = viem_core_view_wait_for_syntax;
static ViemStatus (*font_en_width)(ViemCoreHandle, ViemViewId, uint64_t, float *) = viem_core_view_font_en_width;
static ViemStatus (*navigate_link_fragment)(ViemCoreHandle, ViemViewId, uint64_t, uint64_t, ViemUtf8Slice, uint8_t *, ViemCoreOutcome *) = viem_core_view_navigate_link_fragment;
static ViemStatus (*edit_image)(ViemCoreHandle, ViemViewId, const ViemLogicalSelectionIdentity *, uint32_t, uint64_t, uint64_t, ViemUtf8Slice, ViemUtf8Slice, uint32_t, uint32_t, ViemCoreOutcome *) = viem_core_view_edit_image;
static ViemStatus (*strikethrough_state)(ViemCoreHandle, ViemViewId, uint32_t *) = viem_core_view_strikethrough_state;
static ViemStatus (*set_strikethrough)(ViemCoreHandle, ViemViewId, const ViemLogicalSelectionIdentity *, uint8_t, ViemCoreOutcome *) = viem_core_view_set_strikethrough;
_Static_assert(sizeof(ViemTableContext) == {table_context}, "table context size");
_Static_assert(offsetof(ViemTableContext, selection) == {table_context_selection}, "table context selection offset");
_Static_assert(sizeof(ViemInsertTable) == {insert_table}, "insert table size");
_Static_assert(offsetof(ViemInsertTable, columns) == {insert_table_columns}, "insert table dimensions offset");
_Static_assert(sizeof(ViemTableAction) == {table_action}, "table action size");
_Static_assert(offsetof(ViemTableAction, expected) == {table_action_expected}, "table action identity offset");
_Static_assert(sizeof(ViemTableCell) == {table_cell}, "table cell size");
_Static_assert(offsetof(ViemTableCell, rect) == {table_cell_rect}, "table cell geometry offset");
_Static_assert(sizeof(ViemTableSelection) == {table_selection}, "table selection size");
_Static_assert(VIEM_LOGICAL_SELECTION_KIND_CELLS == {table_selection_kind}, "cell selection kind");
_Static_assert(VIEM_STYLE_ROLE_TABLE == {table_role}, "table style role");
_Static_assert(VIEM_STYLE_ROLE_CONTAINER == {container_role}, "container style role");
_Static_assert(VIEM_EX_OPTION_NUMBER == {number_option}, "number option");
_Static_assert(VIEM_STYLE_DEFINITION_LINE_NUMBER == {line_number_style_flag}, "line number definition flag");
_Static_assert(VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER == {current_line_number_style_flag}, "current line number definition flag");
_Static_assert(VIEM_LINE_NUMBER_REGION_CURRENT == {current_line_number_region_flag}, "current line number region flag");
_Static_assert(VIEM_LINE_NUMBER_DIFF_ADDED == {gutter_added} && VIEM_LINE_NUMBER_DIFF_REMOVED == {gutter_removed}, "diff line number underlays");
_Static_assert(VIEM_DIFF_SESSION_ALIGNED == {diff_aligned}, "diff aligned flag");
_Static_assert(VIEM_DIFF_SESSION_COLLAPSED == {diff_collapsed}, "diff collapsed flag");
_Static_assert(VIEM_DIFF_SYNC_INCREMENTAL == {diff_sync_incremental} && VIEM_DIFF_SYNC_FULL == {diff_sync_full}, "diff sync results");
_Static_assert(VIEM_DIFF_BASE_UNSTAGED == {diff_base_unstaged} && VIEM_DIFF_BASE_BRANCH == {diff_base_branch} && VIEM_DIFF_BASE_FILE == {diff_base_file}, "diff base kinds");
_Static_assert(VIEM_DIFF_BASE_COMMIT == {diff_base_commit}, "commit base kind");
_Static_assert(VIEM_DIFF_RESULT_UNAVAILABLE == {diff_result_unavailable} && VIEM_DIFF_RESULT_COMMAND_FAILED == {diff_result_command}, "diff result codes");
_Static_assert(VIEM_DIFF_RESULT_BASE_EXISTS == {diff_base_exists}, "diff base exists flag");
_Static_assert(VIEM_DIFF_FILE_RENAMED == {diff_file_renamed} && VIEM_DIFF_FILE_UNSAVED == {diff_file_unsaved}, "diff file changes");
_Static_assert(VIEM_DIFF_NAVIGATE_PREVIOUS == {diff_navigate_previous} && VIEM_DIFF_NAVIGATION_SCROLLED == {diff_navigation_scrolled}, "diff navigation");
_Static_assert(VIEM_DIFF_NAVIGATION_FOUND == {diff_navigation_found} && VIEM_DIFF_MARK_ADDED == {diff_mark_added}, "diff navigation and marks");
_Static_assert(sizeof(ViemLineNumberSnapshotInfo) == {line_number_info_size}, "line number info size");
_Static_assert(offsetof(ViemLineNumberSnapshotInfo, viewport) == {line_number_info_viewport}, "line number viewport offset");
_Static_assert(sizeof(ViemLineNumberRegion) == {line_number_region_size}, "line number region size");
_Static_assert(offsetof(ViemLineNumberRegion, top) == {line_number_region_top}, "line number geometry offset");
static ViemStatus (*line_number_info)(ViemCoreHandle, ViemViewId, ViemLineNumberSnapshotInfo *) = viem_core_view_line_number_snapshot_info;
static ViemStatus (*line_number_regions)(ViemCoreHandle, ViemViewId, const ViemLayoutSnapshotIdentity *, const ViemLayoutRect *, ViemLineNumberRegion *, uint64_t, uint64_t *) = viem_core_view_copy_line_number_regions;
static ViemStatus (*number_option_get)(ViemCoreHandle, ViemViewId, uint32_t *) = viem_core_view_number;
static ViemStatus (*number_option_set)(ViemCoreHandle, ViemViewId, uint32_t) = viem_core_view_set_number;

static ViemStatus (*prepare_table_refinement)(ViemCoreHandle, ViemViewId, uint64_t *) = viem_core_view_prepare_table_refinement;
static ViemStatus (*install_table_refinement)(ViemCoreHandle, ViemViewId, uint64_t, uint8_t *) = viem_core_view_install_table_refinement;
static ViemStatus (*table_context)(ViemCoreHandle, ViemViewId, ViemTableContext *) = viem_core_view_table_context;
_Static_assert(sizeof(ViemSetBlockQuote) == {set_block_quote}, "quote request");
_Static_assert(offsetof(ViemSetBlockQuote, expected_selection) == {quote_selection}, "quote selection");
static ViemStatus (*set_block_quote)(ViemCoreHandle, ViemViewId, const ViemSetBlockQuote *, ViemCoreOutcome *) = viem_core_view_set_block_quote;
static ViemStatus (*table_context_at)(ViemCoreHandle, ViemViewId, uint64_t, uint64_t, uint64_t, ViemTableContext *) = viem_core_view_table_context_at;
static ViemStatus (*insert_table)(ViemCoreHandle, ViemViewId, const ViemInsertTable *, ViemCoreOutcome *) = viem_core_view_insert_table;
static ViemStatus (*table_action)(ViemCoreHandle, ViemViewId, const ViemTableAction *, ViemCoreOutcome *) = viem_core_view_table_action;
static ViemStatus (*copy_table_cells)(ViemCoreHandle, ViemViewId, const ViemLayoutSnapshotIdentity *, ViemTableCell *, uint64_t, uint64_t *) = viem_core_view_copy_table_cells;
static ViemStatus (*copy_table_selection_text)(ViemCoreHandle, ViemViewId, const ViemTableSelection *, uint8_t *, uint64_t, uint64_t *) = viem_core_view_copy_table_selection_text;
static ViemStatus (*copy_table_selection_ranges)(ViemCoreHandle, ViemViewId, const ViemTableSelection *, ViemFormattedUtf8Range *, uint64_t, uint64_t *) = viem_core_view_copy_table_selection_ranges;
static ViemStatus (*table_selection)(ViemCoreHandle, ViemViewId, ViemTableSelection *) = viem_core_view_table_selection;
static ViemStatus (*select_table_cells)(ViemCoreHandle, ViemViewId, const ViemTableSelection *, ViemCoreOutcome *) = viem_core_view_select_table_cells;
static ViemStatus (*style_defaults)(ViemCoreHandle, uint64_t, const uint8_t *, uint64_t, ViemStyleDefaultsDiagnosticCallback, void *) = viem_core_initialize_style_defaults;
_Static_assert(VIEM_ENCODING_DETECT == 0u, "automatic encoding choice");
_Static_assert(VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION == 4u,
    "provider ABI");
_Static_assert(VIEM_STATUS_UNSTABLE_SHAPING_CONTEXT == 26u,
    "bounded-context refusal status");
_Static_assert(VIEM_STATUS_LAYOUT_UNAVAILABLE == 28u,
    "layout unavailable status");
_Static_assert(VIEM_STATUS_OUTSIDE_LAYOUT_COVERAGE == 29u,
    "outside layout coverage status");
_Static_assert(VIEM_STATUS_INVALID_UTF8_BOUNDARY == 36u,
    "invalid UTF-8 boundary status");
_Static_assert(VIEM_STATUS_INVALID_UTF16_BOUNDARY == 37u,
    "invalid UTF-16 boundary status");
_Static_assert(VIEM_STATUS_UNFINISHED_COMMENT == 41u,
    "unfinished-comment status must remain 41");
_Static_assert(VIEM_STATUS_STYLE_EDIT_GROUP_ACTIVE == 38u,
    "active style edit group status");
_Static_assert(VIEM_STATUS_INVALID_STYLE_EDIT_GROUP == 39u,
    "invalid style edit group status");
_Static_assert(VIEM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER == 40u,
    "style edit group owner status");
_Static_assert(VIEM_VIEWPORT_ORIGIN_HAS_TOP == (1u << 0),
    "viewport top request flag");
_Static_assert(VIEM_VIEWPORT_STATE_WRAP == (1u << 0), "viewport wrap flag");
_Static_assert(VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT == (1u << 1),
    "viewport maximum-left exact flag");
_Static_assert(VIEM_VIEWPORT_STATE_TOP_EXACT == (1u << 2),
    "viewport top exact flag");
_Static_assert(VIEM_VIEWPORT_STATE_HAS_LAYOUT == (1u << 3),
    "viewport layout identity flag");
_Static_assert(VIEM_VIEWPORT_STATE_LINEBREAK == (1u << 4),
    "viewport linebreak flag");
_Static_assert(VIEM_FILE_FORMAT_ORIGIN_DETECTED == 1u,
    "detected file-format origin");
_Static_assert(VIEM_FILE_FORMAT_ORIGIN_FORCED == 2u,
    "forced file-format origin");
_Static_assert(VIEM_FILE_FORMAT_ORIGIN_DEFAULTED == 3u,
    "defaulted file-format origin");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_NONE == 0u,
    "no history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_TEXT == 1u,
    "text history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_STYLE == 2u,
    "style history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT == 3u,
    "file-format history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER == 4u,
    "hard-line transfer history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA == 6u,
    "source-metadata history action category");
_Static_assert(VIEM_HISTORY_ACTION_CATEGORY_MIXED == 7u,
    "mixed history action category");
_Static_assert(VIEM_DOCUMENT_STATE_HAS_BOM == (1u << 0),
    "document BOM flag");
_Static_assert(VIEM_DOCUMENT_STATE_CAN_UNDO == (1u << 1),
    "document can-undo flag");
_Static_assert(VIEM_DOCUMENT_STATE_CAN_REDO == (1u << 2),
    "document can-redo flag");
_Static_assert(VIEM_DOCUMENT_STATE_IS_DIRTY == (1u << 3),
    "document dirty flag");
_Static_assert(VIEM_CLIPBOARD_TARGET_CLIPBOARD == 1u,
    "clipboard target");
_Static_assert(VIEM_CLIPBOARD_TARGET_PRIMARY == 2u,
    "primary target");
_Static_assert(VIEM_CLIPBOARD_TURN_HAS_READ == (1u << 0),
    "clipboard read flag");
_Static_assert(VIEM_CLIPBOARD_TURN_WRITABLE == (1u << 1),
    "clipboard writable flag");
_Static_assert(VIEM_EFFECT_BATCH_HAS_EX_OUTCOME == (1u << 0),
    "effect Ex outcome flag");
_Static_assert(VIEM_EFFECT_BATCH_EX_DOCUMENT_CHANGED == (1u << 1),
    "effect document-changed flag");
_Static_assert(VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION == (1u << 2),
    "effect navigation flag");
_Static_assert(VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY == (1u << 3),
    "effect history-navigation flag");
_Static_assert(VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER == (1u << 0),
    "portable clipboard flag");
_Static_assert(VIEM_REGISTER_KIND_NONE == 0u, "no register shape");
_Static_assert(VIEM_REGISTER_KIND_CHARACTER == 1u, "character register");
_Static_assert(VIEM_REGISTER_KIND_LINE == 2u, "line register");
_Static_assert(VIEM_REGISTER_KIND_BLOCK == 3u, "block register");
_Static_assert(VIEM_EX_FRONTEND_EDIT == 1u, "Ex edit request");
_Static_assert(VIEM_EX_FRONTEND_NORMAL == 15u, "Ex normal request");
_Static_assert(VIEM_EX_FRONTEND_THEME == 30u, "Ex theme request");
_Static_assert(VIEM_EX_FRONTEND_APPLICATION_OPTION == 31u, "Ex application option request");
_Static_assert(VIEM_EX_FRONTEND_SCRIPT_CONTINUATION == 32u, "Script continuation request");
_Static_assert(VIEM_EX_FRONTEND_EDIT_ALTERNATE == 33u, "Ex alternate-file request");
_Static_assert(VIEM_EX_FRONTEND_STARTUP_APPLICATION == (1u << 15), "Startup application flag");
_Static_assert(VIEM_EX_FRONTEND_HAS_PATH == (1u << 1), "Ex path flag");
_Static_assert(VIEM_EX_FRONTEND_HAS_RANGE == (1u << 2), "Ex range flag");
_Static_assert(VIEM_MODE_SELECTION_CHARACTER == {selection_character}u, "native Selection character mode");
_Static_assert(VIEM_MODE_SELECTION_LINE == {selection_line}u, "native Selection line mode");
_Static_assert(VIEM_MODE_SELECTION_BLOCK == {selection_block}u, "native Selection block mode");
_Static_assert(VIEM_EX_OPTION_AUTOSELECT == {autoselect}u, "autoselect option");
_Static_assert(VIEM_EX_OPTION_WRAP == 1u, "Ex wrap option");
_Static_assert(VIEM_EX_OPTION_FILE_FORMATS == 4u, "Ex fileformats option");
_Static_assert(VIEM_EX_OPTION_VALUE_BOOLEAN == 1u, "Ex Boolean value");
_Static_assert(VIEM_EX_OPTION_VALUE_FILE_FORMATS == 3u,
    "Ex fileformats value");
_Static_assert(VIEM_EX_JUMP_CURRENT == (1u << 0), "current Ex jump");
_Static_assert(VIEM_TEXT_PAINT_HAS_BACKGROUND == (1u << 0),
    "paint background flag");
_Static_assert(VIEM_TEXT_PAINT_UNDERLINE == (1u << 1),
    "paint underline flag");
_Static_assert(VIEM_TEXT_PAINT_STRIKETHROUGH == (1u << 2),
    "paint strikethrough flag");
_Static_assert(VIEM_COMMAND_LINE_KIND_NONE == 0u, "no command line kind");
_Static_assert(VIEM_COMMAND_LINE_KIND_EX == 1u, "Ex command line kind");
_Static_assert(VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD == 2u,
    "forward-search command line kind");
_Static_assert(VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD == 3u,
    "backward-search command line kind");
_Static_assert(VIEM_VISUAL_SELECTION_KIND_NONE == 0u,
    "no Visual selection kind");
_Static_assert(VIEM_VISUAL_SELECTION_KIND_CHARACTER == 1u,
    "Characterwise Visual selection kind");
_Static_assert(VIEM_VISUAL_SELECTION_KIND_LINE == 2u,
    "Linewise Visual selection kind");
_Static_assert(VIEM_VISUAL_SELECTION_KIND_BLOCK == 3u,
    "Blockwise Visual selection kind");
_Static_assert(VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW == (1u << 0),
    "Visual segment row flag");
_Static_assert(VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE == (1u << 1),
    "Visual segment hard-line flag");
_Static_assert(VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES == (1u << 2),
    "Visual segment affinity flag");
_Static_assert(sizeof(ViemDocumentOptions) == {document_options}, "document options");
_Static_assert(sizeof(ViemDocumentState) == {document_state}, "document state");
_Static_assert(sizeof(ViemFormattedSnapshotIdentity) == {formatted_identity},
    "formatted snapshot identity");
_Static_assert(offsetof(ViemFormattedSnapshotIdentity, document_id) ==
    {formatted_identity_document}, "formatted identity document offset");
_Static_assert(sizeof(ViemFormattedSnapshotInfo) == {formatted_info},
    "formatted snapshot info");
_Static_assert(offsetof(ViemFormattedSnapshotInfo, utf8_length) ==
    {formatted_info_utf8}, "formatted info UTF-8 length offset");
_Static_assert(sizeof(ViemFormattedUtf8Range) == {formatted_range},
    "formatted UTF-8 range");
_Static_assert(offsetof(ViemFormattedUtf8Range, utf8_start) ==
    {formatted_range_start}, "formatted range start offset");
_Static_assert(sizeof(ViemFormattedPointInfo) == {formatted_point},
    "formatted point info");
_Static_assert(offsetof(ViemFormattedPointInfo, grapheme_column) ==
    {formatted_point_column}, "formatted point column offset");
_Static_assert(sizeof(ViemClipboardTurnEntry) == {clipboard_turn_entry},
    "clipboard turn entry");
_Static_assert(offsetof(ViemClipboardTurnEntry, plain_text) ==
    {clipboard_turn_text}, "clipboard turn text offset");
_Static_assert(sizeof(ViemFileMarkTarget) == {file_mark_target}, "file mark target");
static ViemStatus (*file_mark_target)(uint32_t, uint64_t, ViemFileMarkTarget *, uint8_t *, uint64_t) = viem_file_mark_target;
static ViemStatus (*jump_file_mark)(ViemCoreHandle, ViemViewId, uint32_t, uint64_t, uint32_t, ViemCoreOutcome *) = viem_core_view_jump_file_mark;
static ViemStatus (*host_register)(ViemCoreHandle, ViemViewId, uint32_t, const uint8_t *, uint64_t) = viem_core_view_set_host_register;
static ViemStatus (*drop_text)(ViemCoreHandle, ViemViewId, const uint8_t *, uint64_t, const ViemCommandTurnContext *, ViemCoreOutcome *, ViemEffectBatchHandle *) = viem_core_view_drop_text;
_Static_assert(offsetof(ViemFileMarkTarget, generation) == {file_mark_generation}, "file mark generation offset");
_Static_assert(sizeof(ViemCommandTurnContext) == {command_turn_context},
    "command turn context");
_Static_assert(sizeof(ViemEffectBytesRef) == {effect_bytes_ref},
    "effect byte reference");
_Static_assert(sizeof(ViemClipboardWrite) == {clipboard_write},
    "clipboard write");
_Static_assert(sizeof(ViemExOptionDisplay) == {ex_option}, "Ex option");
_Static_assert(sizeof(ViemExMark) == {ex_mark}, "Ex mark");
_Static_assert(sizeof(ViemExRegister) == {ex_register}, "Ex register");
_Static_assert(sizeof(ViemExJump) == {ex_jump}, "Ex jump");
_Static_assert(sizeof(ViemExTextLine) == {ex_text_line}, "Ex text line");
_Static_assert(sizeof(ViemPaneChrome) == {pane_chrome}, "Pane chrome");
_Static_assert(sizeof(ViemPaneFrame) == {pane_frame}, "Pane frame");
_Static_assert(offsetof(ViemPaneFrame, x) == {pane_frame_x}, "Pane coordinate alignment");
_Static_assert(sizeof(ViemPaneSnapshot) == {pane_snapshot}, "Pane snapshot");
_Static_assert(sizeof(ViemExFrontendRequest) == {ex_request},
    "Ex frontend request");
_Static_assert(offsetof(ViemExFrontendRequest, first_payload) ==
    {ex_request_payload}, "Ex request payload offset");
_Static_assert(sizeof(ViemEffectBatchInfo) == {effect_batch_info},
    "effect batch info");
_Static_assert(offsetof(ViemEffectBatchInfo, ex_mark_count) ==
    {effect_batch_mark_count}, "effect batch mark count offset");
_Static_assert(sizeof(ViemResolvedTextStyle) == {style}, "style");
_Static_assert(offsetof(ViemResolvedTextStyle, baseline_offset) == {baseline_offset}, "baseline offset");
_Static_assert(VIEM_STYLE_VALUE_PERCENTAGE == {percentage_value}, "percentage value tag");
_Static_assert(sizeof(ViemShapeStyleRun) == {style_run}, "style run");
_Static_assert(sizeof(ViemShapedCluster) == {cluster}, "cluster");
_Static_assert(sizeof(ViemShapingDiagnostic) == {diagnostic}, "diagnostic");
_Static_assert(sizeof(ViemShapeRequest) == {request}, "request");
_Static_assert(sizeof(ViemInlineImage) == {inline_image}, "inline image");
_Static_assert(offsetof(ViemInlineImage, destination) == {inline_image_destination}, "image destination");
_Static_assert(offsetof(ViemShapeRequest, inline_images) == {request_images}, "request images");
_Static_assert(offsetof(ViemShapeRequest, inline_image_count) == {request_image_count}, "request image count");
_Static_assert(sizeof(ViemShapeResponse) == {response}, "response");
_Static_assert(sizeof(ViemTextMeasurementProvider) == {provider}, "provider");
_Static_assert(sizeof(ViemViewOptions) == {view_options}, "view options");
_Static_assert(sizeof(ViemViewportOrigin) == {viewport_origin}, "viewport origin");
_Static_assert(sizeof(ViemViewportState) == {viewport_state}, "viewport state");
_Static_assert(sizeof(ViemLayoutSnapshotIdentity) == {layout_identity},
    "layout identity");
_Static_assert(sizeof(ViemLayoutInsets) == {layout_insets}, "layout insets");
_Static_assert(sizeof(ViemLayoutRect) == {layout_rect}, "layout rect");
_Static_assert(sizeof(ViemRgba) == {rgba}, "RGBA");
_Static_assert(sizeof(ViemStyleSheetIdentity) == {style_sheet_identity},
    "style-sheet identity");
_Static_assert(sizeof(ViemStyleStringRef) == {style_string_ref},
    "style string reference");
_Static_assert(sizeof(ViemStyleSheetInfo) == {style_sheet_info},
    "style-sheet info");
_Static_assert(sizeof(ViemStyleValue) == {style_value}, "style value");
_Static_assert(sizeof(ViemStyleValueItem) == {style_value_item},
    "style value item");
_Static_assert(sizeof(ViemStyleDependency) == {style_dependency},
    "style dependency");
_Static_assert(sizeof(ViemStyleDefinition) == {style_definition},
    "style definition");
_Static_assert(sizeof(ViemStyleProperty) == {style_property},
    "style property");
_Static_assert(sizeof(ViemStyleEditValueItem) == {style_edit_value_item},
    "style edit value item");
_Static_assert(sizeof(ViemStyleEditValue) == {style_edit_value},
    "style edit value");
_Static_assert(sizeof(ViemStyleEdit) == {style_edit}, "style edit");
_Static_assert(sizeof(ViemStyleEditGroup) == {style_edit_group},
    "style edit group");
_Static_assert(offsetof(ViemStyleEditGroup, token) == {style_edit_group_token},
    "style edit group token offset");
_Static_assert(VIEM_STYLE_EDIT_SET_DISPLAY_NAME == 7u,
    "style display-name operation");
_Static_assert(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME == (1u << 3),
    "style display-name capability");
_Static_assert(sizeof(ViemTextPaint) == {text_paint}, "text paint");
_Static_assert(sizeof(ViemLayoutPaintInfo) == {layout_paint_info},
    "layout paint info");
_Static_assert(sizeof(ViemPaintStyleRun) == {paint_style_run},
    "paint style run");
_Static_assert(sizeof(ViemLayoutSnapshotInfo) == {layout_info}, "layout info");
_Static_assert(sizeof(ViemVisualRow) == {visual_row}, "visual row");
_Static_assert(sizeof(ViemPositionedCluster) == {positioned_cluster},
    "positioned cluster");
_Static_assert(sizeof(ViemPositionedCaret) == {positioned_caret},
    "positioned caret");
_Static_assert(sizeof(ViemLayoutCaretRequest) == {caret_request},
    "caret request");
_Static_assert(VIEM_LAYOUT_HIT_TEST_POINTER_DOWN == {pointer_down_flag}, "pointer-down hit-test flag");
_Static_assert(VIEM_PLACE_CURSOR_BEGIN_POINTER_GESTURE == {pointer_gesture_flag}, "pointer gesture flag");
_Static_assert(sizeof(ViemPointerCaret) == {pointer_caret_size}, "pointer caret size");
_Static_assert(offsetof(ViemPointerCaret, text_start) == {pointer_caret_start}, "pointer caret range");
_Static_assert(VIEM_POINTER_CARET_CURRENT == {pointer_caret_current}, "pointer caret current flag");
static ViemStatus (*pointer_caret)(ViemCoreHandle, ViemViewId, const ViemLayoutHitTestRequest *, ViemPointerCaret *) = viem_core_view_pointer_caret;
_Static_assert(offsetof(ViemLayoutHitTestRequest, flags) == {hit_test_flags}, "hit-test flags offset");
_Static_assert(sizeof(ViemLayoutHitTestRequest) == {hit_test_request},
    "hit-test request");
_Static_assert(sizeof(ViemLayoutCaretPoint) == {caret_point}, "caret point");
_Static_assert(sizeof(ViemLayoutCaretGeometry) == {caret_geometry},
    "caret geometry");
_Static_assert(sizeof(ViemViewPresentation) == {presentation},
    "view presentation");
_Static_assert(sizeof(ViemCommandLineIdentity) == {command_line_identity},
    "command-line identity");
_Static_assert(sizeof(ViemCommandLineInfo) == {command_line_info},
    "command-line info");
_Static_assert(sizeof(ViemCompletionInfo) == {completion_info},
    "completion info");
_Static_assert(offsetof(ViemCompletionInfo, selected_index) == {completion_selected},
    "completion signed selection offset");
_Static_assert(offsetof(ViemCompletionInfo, anchor_layout) == {completion_anchor_layout},
    "completion anchor identity offset");
_Static_assert(offsetof(ViemCompletionInfo, anchor_rect) == {completion_anchor_rect},
    "completion anchor geometry offset");
_Static_assert(sizeof(ViemCompletionItem) == {completion_item},
    "completion item");
_Static_assert(VIEM_COMPLETION_ACTIVE == {completion_active}, "completion active flag");
_Static_assert(VIEM_COMPLETION_SEARCHING == {completion_searching}, "completion searching flag");
_Static_assert(VIEM_COMPLETION_TRUNCATED == {completion_truncated}, "completion truncated flag");
_Static_assert(VIEM_COMPLETION_HAS_ANCHOR == {completion_has_anchor}, "completion anchor flag");
_Static_assert(VIEM_COMPLETION_RIGHT_TO_LEFT == {completion_rtl}, "completion RTL flag");
_Static_assert(sizeof(ViemVisualSelectionIdentity) == {visual_selection_identity},
    "Visual-selection identity");
_Static_assert(sizeof(ViemVisualSelectionInfo) == {visual_selection_info},
    "Visual-selection info");
_Static_assert(sizeof(ViemVisualSelectionSegment) == {visual_selection_segment},
    "Visual-selection segment");
_Static_assert(sizeof(ViemVisualSelectionRectangle) == {visual_selection_rectangle},
    "Visual-selection rectangle");
_Static_assert(sizeof(ViemPlaceCursor) == {place_cursor}, "place cursor");
_Static_assert(sizeof(ViemAssignStyle) == {assign_style}, "assign style");
_Static_assert(sizeof(ViemCreateStyle) == {create_style}, "create style");
_Static_assert(sizeof(ViemDeleteStyle) == {delete_style}, "delete style");
_Static_assert(sizeof(ViemSetFileFormat) == {set_file_format},
    "set file format");
_Static_assert(sizeof(ViemMarkSaved) == {mark_saved}, "mark saved");
_Static_assert(sizeof(ViemKeyInput) == {key}, "key");
_Static_assert(sizeof(ViemCompositionBegin) == {composition_begin}, "composition begin");
_Static_assert(sizeof(ViemCompositionUpdate) == {composition_update}, "composition update");
_Static_assert(sizeof(ViemCompositionCommit) == {composition_commit}, "composition commit");
_Static_assert(sizeof(ViemCompositionCancel) == {composition_cancel}, "composition cancel");
_Static_assert(sizeof(ViemCompositionOverlayIdentity) == {composition_overlay_identity},
    "composition overlay identity");
_Static_assert(sizeof(ViemCompositionOverlayInfo) == {composition_overlay_info},
    "composition overlay info");
_Static_assert(sizeof(ViemCompositionOverlayUtf8Range) == {composition_overlay_range},
    "composition overlay range");
_Static_assert(sizeof(ViemCoreOutcome) == {outcome}, "outcome");
static void typecheck(void) {{
  ViemShapeRequest request = {{0}};
  request.paragraph_base_direction = VIEM_TEXT_DIRECTION_AUTO;
  ViemStatus (*create_core)(const uint8_t *, uint64_t,
      const ViemDocumentOptions *, ViemCoreHandle *, uint64_t *) = viem_core_create;
  ViemStatus (*document_state)(ViemCoreHandle, ViemDocumentState *) =
      viem_core_document_state;
  ViemStatus (*formatted_info)(ViemCoreHandle,
      ViemFormattedSnapshotInfo *) = viem_core_formatted_snapshot_info;
  ViemStatus (*copy_formatted_range)(ViemCoreHandle,
      const ViemFormattedUtf8Range *, uint8_t *, uint64_t, uint64_t *) =
      viem_core_copy_formatted_utf8_range;
  ViemStatus (*map_utf8_to_utf16)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentity *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      viem_core_map_formatted_utf8_to_utf16;
  ViemStatus (*map_utf16_to_utf8)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentity *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      viem_core_map_formatted_utf16_to_utf8;
  ViemStatus (*formatted_point)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentity *, uint64_t,
      ViemFormattedPointInfo *) = viem_core_formatted_point_info;
  ViemStatus (*mark_saved)(ViemCoreHandle, const ViemMarkSaved *) =
      viem_core_mark_saved;
  ViemStatus (*prepare_html_export)(ViemCoreHandle, ViemViewId, uint64_t,
      ViemHtmlExportHandle *) = viem_core_prepare_html_export;
  ViemStatus (*render_html_export)(ViemHtmlExportHandle) = viem_html_export_render;
  ViemStatus (*copy_html_export)(ViemHtmlExportHandle, uint8_t *, uint64_t,
      uint64_t *) = viem_html_export_copy_utf8;
  ViemStatus (*release_html_export)(ViemHtmlExportHandle) = viem_html_export_release;
  ViemStatus (*diff_create)(ViemCoreHandle, ViemCoreHandle, uint32_t, ViemDiffSessionHandle *) =
      viem_diff_session_create;
  ViemStatus (*diff_sync)(ViemDiffSessionHandle, ViemCoreHandle, ViemViewId, ViemCoreHandle,
      ViemViewId, uint32_t *) = viem_diff_session_sync;
  ViemStatus (*diff_line)(ViemDiffSessionHandle, uint32_t, uint64_t, uint64_t *) =
      viem_diff_session_corresponding_line;
  ViemStatus (*diff_load)(const ViemDiffBaseRequest *, ViemDiffResultHandle *) = viem_diff_load_base;
  ViemStatus (*diff_item)(ViemDiffResultHandle, uint64_t, ViemDiffListItem *, uint8_t *, uint64_t,
      uint64_t *) = viem_diff_result_copy_item;
  ViemStatus (*saved_source)(ViemCoreHandle, uint8_t *, uint64_t, uint64_t *, uint32_t *) =
      viem_core_copy_saved_source;
  ViemStatus (*diff_navigate)(ViemDiffSessionHandle, ViemCoreHandle, ViemViewId, ViemCoreHandle,
      ViemViewId, uint32_t, ViemDiffNavigation *) = viem_diff_session_navigate;
  ViemStatus (*diff_marks)(ViemDiffSessionHandle, ViemCoreHandle, ViemViewId, uint32_t,
      ViemDiffScrollbarMark *, uint64_t, uint64_t *, float *) = viem_diff_session_scrollbar_marks;
  ViemStatus (*diff_set_collapsed)(ViemDiffSessionHandle, uint32_t) = viem_diff_session_set_collapsed;
  ViemStatus (*diff_open_collapsed)(ViemDiffSessionHandle, uint32_t, uint64_t, uint32_t *) =
      viem_diff_session_open_collapsed;
  ViemStatus (*diff_opened_context)(ViemCoreHandle, ViemViewId, uint32_t *) = viem_diff_view_opened_context;
  (void)diff_set_collapsed; (void)diff_open_collapsed; (void)diff_opened_context;
  (void)diff_create; (void)diff_sync; (void)diff_line; (void)diff_load; (void)diff_item; (void)saved_source;
  ViemStatus (*diff_colors)(ViemRgba *, ViemRgba *) = viem_diff_line_colors;
  (void)diff_navigate; (void)diff_marks; (void)diff_colors;
  ViemStatus (*import_clipboard)(uint32_t, const uint8_t *, uint64_t,
      uint8_t *, uint64_t, uint64_t *) = viem_import_clipboard_json;
  ViemStatus (*style_sheet_info)(ViemCoreHandle, uint32_t, ViemStyleSheetInfo *) =
      viem_core_style_sheet_info;
  ViemStatus (*copy_style_sheet)(ViemCoreHandle, uint32_t,
      const ViemStyleSheetIdentity *, ViemStyleDefinition *, uint64_t,
      ViemStyleProperty *, uint64_t, ViemStyleValueItem *, uint64_t,
      ViemStyleDependency *, uint64_t, uint8_t *, uint64_t,
      ViemStyleSheetInfo *) = viem_core_copy_style_sheet;
  ViemStatus (*add_view)(ViemCoreHandle, const ViemViewOptions *,
      const ViemTextMeasurementProvider *, ViemViewId *,
      ViemCoreOutcome *) = viem_core_view_add;
  ViemStatus (*viewport_state)(ViemCoreHandle, ViemViewId,
      ViemViewportState *) = viem_core_view_viewport_state;
  ViemStatus (*layout_info)(ViemCoreHandle, ViemViewId,
      ViemLayoutSnapshotInfo *) = viem_core_view_layout_snapshot_info;
  ViemStatus (*copy_layout)(ViemCoreHandle, ViemViewId,
      const ViemLayoutSnapshotIdentity *, ViemVisualRow *, uint64_t,
      ViemPositionedCluster *, uint64_t, ViemPositionedCaret *, uint64_t,
      ViemLayoutSnapshotInfo *) = viem_core_view_copy_layout_snapshot;
  ViemStatus (*layout_paint_info)(ViemCoreHandle, ViemViewId,
      ViemLayoutPaintInfo *) = viem_core_view_layout_paint_info;
  ViemStatus (*copy_layout_paint)(ViemCoreHandle, ViemViewId,
      const ViemLayoutSnapshotIdentity *, ViemPaintStyleRun *, uint64_t,
      ViemLayoutPaintInfo *) = viem_core_view_copy_layout_paint;
  ViemStatus (*caret_geometry)(ViemCoreHandle, ViemViewId,
      const ViemLayoutCaretRequest *, ViemLayoutCaretGeometry *) =
      viem_core_view_caret_geometry;
  ViemStatus (*hit_test)(ViemCoreHandle, ViemViewId,
      const ViemLayoutHitTestRequest *, ViemLayoutCaretPoint *) =
      viem_core_view_layout_hit_test;
  ViemStatus (*presentation)(ViemCoreHandle, ViemViewId,
      ViemViewPresentation *) = viem_core_view_presentation;
  ViemStatus (*command_line_info)(ViemCoreHandle, ViemViewId,
      ViemCommandLineInfo *) = viem_core_view_command_line_info;
  ViemStatus (*copy_command_line)(ViemCoreHandle, ViemViewId,
      const ViemCommandLineIdentity *, uint8_t *, uint64_t,
      ViemCommandLineInfo *) = viem_core_view_copy_command_line;
  ViemStatus (*completion_info)(ViemCoreHandle, ViemViewId,
      ViemCompletionInfo *) = viem_core_view_completion_info;
  ViemStatus (*copy_completion_items)(ViemCoreHandle, ViemViewId,
      const ViemCompletionInfo *, ViemCompletionItem *, uint64_t,
      uint64_t *) = viem_core_view_copy_completion_items;
  ViemStatus (*copy_completion_utf8)(ViemCoreHandle, ViemViewId,
      const ViemCompletionInfo *, uint8_t *, uint64_t,
      uint64_t *) = viem_core_view_copy_completion_utf8;
  ViemStatus (*poll_completion)(ViemCoreHandle, ViemViewId,
      uint8_t *) = viem_core_view_poll_completion;
  ViemStatus (*accept_completion)(ViemCoreHandle, ViemViewId,
      uint8_t *) = viem_core_view_accept_completion;
  ViemStatus (*visual_selection_info)(ViemCoreHandle, ViemViewId,
      ViemVisualSelectionInfo *) = viem_core_view_visual_selection_info;
  ViemStatus (*copy_visual_selection)(ViemCoreHandle, ViemViewId,
      const ViemVisualSelectionIdentity *,
      ViemVisualSelectionSegment *, uint64_t,
      ViemVisualSelectionRectangle *, uint64_t,
      ViemVisualSelectionInfo *) = viem_core_view_copy_visual_selection;
  ViemStatus (*use_selection_for_find)(ViemCoreHandle, ViemViewId,
      const ViemVisualSelectionIdentity *, ViemCoreOutcome *) =
      viem_core_view_use_selection_for_find;
  ViemStatus (*reveal_selection)(ViemCoreHandle, ViemViewId,
      ViemCoreOutcome *) = viem_core_view_reveal_selection;
  ViemStatus (*set_viewport_origin)(ViemCoreHandle, ViemViewId,
      const ViemViewportOrigin *, ViemCoreOutcome *) =
      viem_core_view_set_viewport_origin;
  ViemStatus (*set_scale)(ViemCoreHandle, ViemViewId, float,
      ViemCoreOutcome *) = viem_core_view_set_scale;
  ViemStatus (*set_file_format)(ViemCoreHandle, ViemViewId,
      const ViemSetFileFormat *, ViemCoreOutcome *) =
      viem_core_view_set_file_format;
  ViemStatus (*edit_style)(ViemCoreHandle, ViemViewId,
      const ViemStyleEdit *, ViemCoreOutcome *) =
      viem_core_view_edit_style;
  ViemStatus (*assign_style)(ViemCoreHandle, ViemViewId,
      const ViemAssignStyle *, ViemCoreOutcome *) = viem_core_view_assign_style;
  ViemStatus (*begin_style_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleSheetIdentity *, ViemStyleEditGroup *) =
      viem_core_view_begin_style_edit_group;
  ViemStatus (*edit_style_in_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleEditGroup *, const ViemStyleEdit *,
      ViemCoreOutcome *) = viem_core_view_edit_style_in_group;
  ViemStatus (*end_style_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleEditGroup *) = viem_core_view_end_style_edit_group;
  ViemStatus (*send_key_with_host_context)(ViemCoreHandle, ViemViewId,
      const ViemKeyInput *, const ViemCommandTurnContext *,
      ViemCoreOutcome *, ViemEffectBatchHandle *) =
      viem_core_view_send_key_with_host_context;
  ViemStatus (*send_text_with_host_context)(ViemCoreHandle, ViemViewId,
      const uint8_t *, uint64_t, const ViemCommandTurnContext *,
      ViemCoreOutcome *, ViemEffectBatchHandle *) =
      viem_core_view_send_text_with_host_context;
  ViemStatus (*effect_info)(ViemEffectBatchHandle,
      ViemEffectBatchInfo *) = viem_effect_batch_info;
  ViemStatus (*effect_read)(ViemEffectBatchHandle,
      ViemEffectBatchView *) = viem_effect_batch_read;
  ViemStatus (*effect_release)(ViemEffectBatchHandle) =
      viem_effect_batch_release;
  ViemStatus (*place_cursor)(ViemCoreHandle, ViemViewId,
      const ViemPlaceCursor *, ViemCoreOutcome *) =
      viem_core_view_place_cursor;
  ViemStatus (*undo)(ViemCoreHandle, ViemViewId, ViemCoreOutcome *) =
      viem_core_view_undo;
  ViemStatus (*redo)(ViemCoreHandle, ViemViewId, ViemCoreOutcome *) =
      viem_core_view_redo;
  ViemStatus (*composition_begin)(ViemCoreHandle, ViemViewId,
      const ViemCompositionBegin *, ViemCoreOutcome *) =
      viem_core_view_composition_begin;
  ViemStatus (*composition_update)(ViemCoreHandle, ViemViewId,
      const ViemCompositionUpdate *, ViemCoreOutcome *) =
      viem_core_view_composition_update;
  ViemStatus (*composition_overlay_info)(ViemCoreHandle, ViemViewId,
      ViemCompositionOverlayInfo *) =
      viem_core_view_composition_overlay_info;
  ViemStatus (*copy_composition_range)(ViemCoreHandle, ViemViewId,
      const ViemCompositionOverlayUtf8Range *, uint8_t *, uint64_t,
      uint64_t *) = viem_core_view_copy_composition_utf8_range;
  ViemStatus (*composition_commit)(ViemCoreHandle, ViemViewId,
      const ViemCompositionCommit *, ViemCoreOutcome *) =
      viem_core_view_composition_commit;
  ViemStatus (*composition_cancel)(ViemCoreHandle, ViemViewId,
      const ViemCompositionCancel *, ViemCoreOutcome *) =
      viem_core_view_composition_cancel;
  (void)request; (void)create_core; (void)document_state;
  (void)formatted_info; (void)copy_formatted_range;
  (void)map_utf8_to_utf16; (void)map_utf16_to_utf8;
  (void)formatted_point; (void)mark_saved;
  (void)style_sheet_info; (void)copy_style_sheet;
  (void)add_view; (void)viewport_state;
  (void)layout_info; (void)copy_layout; (void)layout_paint_info;
  (void)copy_layout_paint; (void)caret_geometry; (void)hit_test;
  (void)presentation; (void)command_line_info; (void)copy_command_line;
  (void)completion_info; (void)copy_completion_items;
  (void)copy_completion_utf8; (void)poll_completion;
  (void)accept_completion;
  (void)visual_selection_info; (void)copy_visual_selection;
  (void)use_selection_for_find; (void)reveal_selection;
  (void)set_viewport_origin; (void)set_scale;
  (void)set_file_format;
  (void)prepare_html_export; (void)render_html_export; (void)copy_html_export;
  (void)release_html_export; (void)import_clipboard;
  (void)edit_style; (void)begin_style_group; (void)edit_style_in_group;
  (void)assign_style;
  (void)end_style_group;

  (void)send_key_with_host_context;
  (void)send_text_with_host_context; (void)effect_info;
  (void)effect_read; (void)effect_release;
  (void)place_cursor; (void)undo; (void)redo;
  (void)composition_begin; (void)composition_update;
  (void)composition_overlay_info; (void)copy_composition_range;
  (void)composition_commit; (void)composition_cancel;
  ViemStatus (*layout_diagnostics)(ViemCoreHandle, ViemViewId,
      const ViemLayoutSnapshotIdentity *, uint8_t *, uint64_t, uint64_t *) =
      viem_core_view_copy_layout_diagnostics;
  (void)layout_diagnostics;
}}
"#,
        table_context = std::mem::size_of::<ViemTableContext>(),
        table_context_selection = std::mem::offset_of!(ViemTableContext, selection),
        insert_table = std::mem::size_of::<ViemInsertTable>(),
        insert_table_columns = std::mem::offset_of!(ViemInsertTable, columns),
        table_action = std::mem::size_of::<ViemTableAction>(),
        table_action_expected = std::mem::offset_of!(ViemTableAction, expected),
        table_cell = std::mem::size_of::<ViemTableCell>(),
        table_cell_rect = std::mem::offset_of!(ViemTableCell, rect),
        table_selection = std::mem::size_of::<ViemTableSelection>(),
        table_selection_kind = VIEM_LOGICAL_SELECTION_KIND_CELLS,
        table_role = VIEM_STYLE_ROLE_TABLE,
        container_role = VIEM_STYLE_ROLE_CONTAINER,
        number_option = VIEM_EX_OPTION_NUMBER,
        line_number_style_flag = VIEM_STYLE_DEFINITION_LINE_NUMBER,
        current_line_number_style_flag = VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER,
        current_line_number_region_flag = VIEM_LINE_NUMBER_REGION_CURRENT,
        gutter_added = VIEM_LINE_NUMBER_DIFF_ADDED,
        gutter_removed = VIEM_LINE_NUMBER_DIFF_REMOVED,
        diff_aligned = VIEM_DIFF_SESSION_ALIGNED,
        diff_collapsed = VIEM_DIFF_SESSION_COLLAPSED,
        diff_sync_incremental = VIEM_DIFF_SYNC_INCREMENTAL,
        diff_sync_full = VIEM_DIFF_SYNC_FULL,
        diff_base_unstaged = VIEM_DIFF_BASE_UNSTAGED,
        diff_base_branch = VIEM_DIFF_BASE_BRANCH,
        diff_base_commit = VIEM_DIFF_BASE_COMMIT,
        diff_base_file = VIEM_DIFF_BASE_FILE,
        diff_result_unavailable = VIEM_DIFF_RESULT_UNAVAILABLE,
        diff_result_command = VIEM_DIFF_RESULT_COMMAND_FAILED,
        diff_base_exists = VIEM_DIFF_RESULT_BASE_EXISTS,
        diff_file_renamed = VIEM_DIFF_FILE_RENAMED,
        diff_file_unsaved = VIEM_DIFF_FILE_UNSAVED,
        diff_navigate_previous = VIEM_DIFF_NAVIGATE_PREVIOUS,
        diff_navigation_scrolled = VIEM_DIFF_NAVIGATION_SCROLLED,
        diff_navigation_found = VIEM_DIFF_NAVIGATION_FOUND,
        diff_mark_added = VIEM_DIFF_MARK_ADDED,
        line_number_info_size = std::mem::size_of::<ViemLineNumberSnapshotInfo>(),
        line_number_info_viewport = std::mem::offset_of!(ViemLineNumberSnapshotInfo, viewport),
        line_number_region_size = std::mem::size_of::<ViemLineNumberRegion>(),
        line_number_region_top = std::mem::offset_of!(ViemLineNumberRegion, top),
        abi = VIEM_CORE_ABI_VERSION,
        alt_enter = VIEM_KEY_ALT_ENTER,
        language_picker_disabled = VIEM_LAYOUT_DECORATION_LANGUAGE_PICKER_DISABLED,
        checkbox_decoration = VIEM_LAYOUT_DECORATION_CHECKBOX,
        checked_decoration = VIEM_LAYOUT_DECORATION_CHECKED,
        task_style = VIEM_LIST_STYLE_TASK,
        has_table = VIEM_SELECTED_STYLE_HAS_TABLE,
        alert_icon = VIEM_LAYOUT_DECORATION_ALERT_ICON,
        alert_title = VIEM_LAYOUT_DECORATION_ALERT_TITLE,
        alert_kind_shift = VIEM_LAYOUT_DECORATION_ALERT_KIND_SHIFT,
        collapsed_context = VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT,
        alert_caution = VIEM_ALERT_CAUTION,
        formatting_bold = VIEM_FORMATTING_BOLD,
        formatting_alert_note = VIEM_FORMATTING_ALERT_NOTE,
        formatting_heading_0 = VIEM_FORMATTING_HEADING_0,
        formatting_character_styles = VIEM_FORMATTING_CHARACTER_STYLES,
        list_paragraph_capability = VIEM_LIST_CAN_INSERT_PARAGRAPH,
        selection_character = VIEM_MODE_SELECTION_CHARACTER,
        selection_line = VIEM_MODE_SELECTION_LINE,
        selection_block = VIEM_MODE_SELECTION_BLOCK,
        autoselect = VIEM_EX_OPTION_AUTOSELECT,
        set_mode_size = std::mem::size_of::<ViemSetDocumentMode>(),
        document_options = std::mem::size_of::<ViemDocumentOptions>(),
        document_state = std::mem::size_of::<ViemDocumentState>(),
        formatted_identity = std::mem::size_of::<ViemFormattedSnapshotIdentity>(),
        formatted_identity_document =
            std::mem::offset_of!(ViemFormattedSnapshotIdentity, document_id),
        formatted_info = std::mem::size_of::<ViemFormattedSnapshotInfo>(),
        formatted_info_utf8 = std::mem::offset_of!(ViemFormattedSnapshotInfo, utf8_length),
        formatted_range = std::mem::size_of::<ViemFormattedUtf8Range>(),
        formatted_range_start = std::mem::offset_of!(ViemFormattedUtf8Range, utf8_start),
        formatted_point = std::mem::size_of::<ViemFormattedPointInfo>(),
        formatted_point_column = std::mem::offset_of!(ViemFormattedPointInfo, grapheme_column),
        clipboard_turn_entry = std::mem::size_of::<ViemClipboardTurnEntry>(),
        clipboard_turn_text = std::mem::offset_of!(ViemClipboardTurnEntry, plain_text),
        file_mark_target = std::mem::size_of::<ViemFileMarkTarget>(),
        file_mark_generation = std::mem::offset_of!(ViemFileMarkTarget, generation),
        command_turn_context = std::mem::size_of::<ViemCommandTurnContext>(),
        effect_bytes_ref = std::mem::size_of::<ViemEffectBytesRef>(),
        clipboard_write = std::mem::size_of::<ViemClipboardWrite>(),
        ex_option = std::mem::size_of::<ViemExOptionDisplay>(),
        ex_mark = std::mem::size_of::<ViemExMark>(),
        ex_register = std::mem::size_of::<ViemExRegister>(),
        ex_jump = std::mem::size_of::<ViemExJump>(),
        ex_text_line = std::mem::size_of::<ViemExTextLine>(),
        pane_chrome = std::mem::size_of::<ViemPaneChrome>(),
        pane_frame = std::mem::size_of::<ViemPaneFrame>(),
        pane_frame_x = std::mem::offset_of!(ViemPaneFrame, x),
        pane_snapshot = std::mem::size_of::<ViemPaneSnapshot>(),
        ex_request = std::mem::size_of::<ViemExFrontendRequest>(),
        ex_request_payload = std::mem::offset_of!(ViemExFrontendRequest, first_payload),
        effect_batch_info = std::mem::size_of::<ViemEffectBatchInfo>(),
        effect_batch_mark_count = std::mem::offset_of!(ViemEffectBatchInfo, ex_mark_count),
        style = std::mem::size_of::<ViemResolvedTextStyle>(),
        baseline_offset = std::mem::offset_of!(ViemResolvedTextStyle, baseline_offset),
        percentage_value = VIEM_STYLE_VALUE_PERCENTAGE,
        style_run = std::mem::size_of::<ViemShapeStyleRun>(),
        cluster = std::mem::size_of::<ViemShapedCluster>(),
        diagnostic = std::mem::size_of::<ViemShapingDiagnostic>(),
        request = std::mem::size_of::<ViemShapeRequest>(),
        inline_image = std::mem::size_of::<ViemInlineImage>(),
        inline_image_destination = std::mem::offset_of!(ViemInlineImage, destination),
        request_images = std::mem::offset_of!(ViemShapeRequest, inline_images),
        request_image_count = std::mem::offset_of!(ViemShapeRequest, inline_image_count),
        response = std::mem::size_of::<ViemShapeResponse>(),
        provider = std::mem::size_of::<ViemTextMeasurementProvider>(),
        view_options = std::mem::size_of::<ViemViewOptions>(),
        viewport_origin = std::mem::size_of::<ViemViewportOrigin>(),
        viewport_state = std::mem::size_of::<ViemViewportState>(),
        layout_identity = std::mem::size_of::<ViemLayoutSnapshotIdentity>(),
        layout_insets = std::mem::size_of::<ViemLayoutInsets>(),
        layout_rect = std::mem::size_of::<ViemLayoutRect>(),
        rgba = std::mem::size_of::<ViemRgba>(),
        style_sheet_identity = std::mem::size_of::<ViemStyleSheetIdentity>(),
        style_string_ref = std::mem::size_of::<ViemStyleStringRef>(),
        style_sheet_info = std::mem::size_of::<ViemStyleSheetInfo>(),
        style_value = std::mem::size_of::<ViemStyleValue>(),
        style_value_item = std::mem::size_of::<ViemStyleValueItem>(),
        style_dependency = std::mem::size_of::<ViemStyleDependency>(),
        style_definition = std::mem::size_of::<ViemStyleDefinition>(),
        style_property = std::mem::size_of::<ViemStyleProperty>(),
        style_edit_value_item = std::mem::size_of::<ViemStyleEditValueItem>(),
        style_edit_value = std::mem::size_of::<ViemStyleEditValue>(),
        style_edit = std::mem::size_of::<ViemStyleEdit>(),
        style_edit_group = std::mem::size_of::<ViemStyleEditGroup>(),
        style_edit_group_token = std::mem::offset_of!(ViemStyleEditGroup, token),
        text_paint = std::mem::size_of::<ViemTextPaint>(),
        layout_paint_info = std::mem::size_of::<ViemLayoutPaintInfo>(),
        paint_style_run = std::mem::size_of::<ViemPaintStyleRun>(),
        layout_info = std::mem::size_of::<ViemLayoutSnapshotInfo>(),
        visual_row = std::mem::size_of::<ViemVisualRow>(),
        positioned_cluster = std::mem::size_of::<ViemPositionedCluster>(),
        positioned_caret = std::mem::size_of::<ViemPositionedCaret>(),
        caret_request = std::mem::size_of::<ViemLayoutCaretRequest>(),
        pointer_down_flag = VIEM_LAYOUT_HIT_TEST_POINTER_DOWN,
        pointer_gesture_flag = VIEM_PLACE_CURSOR_BEGIN_POINTER_GESTURE,
        pointer_caret_size = std::mem::size_of::<ViemPointerCaret>(),
        pointer_caret_start = std::mem::offset_of!(ViemPointerCaret, text_start),
        pointer_caret_current = VIEM_POINTER_CARET_CURRENT,
        hit_test_flags = std::mem::offset_of!(ViemLayoutHitTestRequest, flags),
        hit_test_request = std::mem::size_of::<ViemLayoutHitTestRequest>(),
        caret_point = std::mem::size_of::<ViemLayoutCaretPoint>(),
        caret_geometry = std::mem::size_of::<ViemLayoutCaretGeometry>(),
        presentation = std::mem::size_of::<ViemViewPresentation>(),
        command_line_identity = std::mem::size_of::<ViemCommandLineIdentity>(),
        command_line_info = std::mem::size_of::<ViemCommandLineInfo>(),
        completion_info = std::mem::size_of::<ViemCompletionInfo>(),
        completion_selected = std::mem::offset_of!(ViemCompletionInfo, selected_index),
        completion_anchor_layout = std::mem::offset_of!(ViemCompletionInfo, anchor_layout),
        completion_anchor_rect = std::mem::offset_of!(ViemCompletionInfo, anchor_rect),
        completion_item = std::mem::size_of::<ViemCompletionItem>(),
        completion_active = VIEM_COMPLETION_ACTIVE,
        completion_searching = VIEM_COMPLETION_SEARCHING,
        completion_truncated = VIEM_COMPLETION_TRUNCATED,
        completion_has_anchor = VIEM_COMPLETION_HAS_ANCHOR,
        completion_rtl = VIEM_COMPLETION_RIGHT_TO_LEFT,
        visual_selection_identity = std::mem::size_of::<ViemVisualSelectionIdentity>(),
        visual_selection_info = std::mem::size_of::<ViemVisualSelectionInfo>(),
        visual_selection_segment = std::mem::size_of::<ViemVisualSelectionSegment>(),
        visual_selection_rectangle = std::mem::size_of::<ViemVisualSelectionRectangle>(),
        place_cursor = std::mem::size_of::<ViemPlaceCursor>(),
        assign_style = std::mem::size_of::<ViemAssignStyle>(),
        set_block_quote = std::mem::size_of::<ViemSetBlockQuote>(),
        quote_selection = std::mem::offset_of!(ViemSetBlockQuote, expected_selection),
        create_style = std::mem::size_of::<ViemCreateStyle>(),
        delete_style = std::mem::size_of::<ViemDeleteStyle>(),
        set_file_format = std::mem::size_of::<ViemSetFileFormat>(),
        mark_saved = std::mem::size_of::<ViemMarkSaved>(),
        key = std::mem::size_of::<ViemKeyInput>(),
        composition_begin = std::mem::size_of::<ViemCompositionBegin>(),
        composition_update = std::mem::size_of::<ViemCompositionUpdate>(),
        composition_commit = std::mem::size_of::<ViemCompositionCommit>(),
        composition_cancel = std::mem::size_of::<ViemCompositionCancel>(),
        composition_overlay_identity = std::mem::size_of::<ViemCompositionOverlayIdentity>(),
        composition_overlay_info = std::mem::size_of::<ViemCompositionOverlayInfo>(),
        composition_overlay_range = std::mem::size_of::<ViemCompositionOverlayUtf8Range>(),
        outcome = std::mem::size_of::<ViemCoreOutcome>(),
    );
    append_generated_layout_checks(&mut source);
    source
}
