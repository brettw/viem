//! Explicit native C ABI validation; see docs/abi-validation.md.

use std::path::Path;
use std::process::{Command, ExitCode};
use viem_core::ffi::*;

fn main() -> ExitCode {
    match check_c_abi() {
        Ok(()) => {
            println!("C header matches the Rust ABI assertions.");
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
    format!(
        r#"
#include "viem_core.h"
#include <stddef.h>
_Static_assert(VIEM_CORE_ABI_VERSION == {abi}, "ABI version");
_Static_assert(sizeof(ViemSetFormatV1) == {set_format_size}, "format operation size");
_Static_assert(offsetof(ViemSetFormatV1, operation) == {set_format_operation}, "format operation offset");
_Static_assert(offsetof(ViemSetFormatV1, document_id) == {set_format_document}, "format document offset");
_Static_assert(VIEM_FORMAT_OPERATION_REINTERPRET == {reinterpret}u, "reinterpret operation");
_Static_assert(VIEM_FORMAT_OPERATION_CONVERT == {convert}u, "convert operation");
_Static_assert(sizeof(ViemDirectStyleEditV1) == {direct_style_edit}, "direct style size");
_Static_assert(_Alignof(ViemDirectStyleEditV1) == {direct_style_align}, "direct style alignment");
_Static_assert(offsetof(ViemDirectStyleEditV1, expected_selection) == {direct_style_selection}, "direct style selection offset");
_Static_assert(offsetof(ViemDirectStyleEditV1, value) == {direct_style_value}, "direct style value offset");
static ViemStatus (*direct_style)(ViemCoreHandle, ViemViewId, const ViemDirectStyleEditV1 *, ViemCoreOutcomeV1 *) = viem_core_view_edit_direct_style;
static ViemStatus (*decoration_state)(ViemCoreHandle, ViemViewId, uint32_t, uint32_t *) = viem_core_view_decoration_state;
_Static_assert(VIEM_ENCODING_DETECT == 0u, "automatic encoding choice");
_Static_assert(VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V3 == 3u,
    "provider ABI v3");
_Static_assert(VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION ==
    VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V3, "current provider ABI");
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
_Static_assert(sizeof(ViemDocumentStateV1) == {document_state}, "document state");
_Static_assert(VIEM_DOCUMENT_STATE_V1_SIZE == sizeof(ViemDocumentStateV1),
    "document state size macro");
_Static_assert(sizeof(ViemFormattedSnapshotIdentityV1) == {formatted_identity},
    "formatted snapshot identity");
_Static_assert(VIEM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE ==
    sizeof(ViemFormattedSnapshotIdentityV1),
    "formatted snapshot identity size macro");
_Static_assert(offsetof(ViemFormattedSnapshotIdentityV1, document_id) ==
    {formatted_identity_document}, "formatted identity document offset");
_Static_assert(sizeof(ViemFormattedSnapshotInfoV1) == {formatted_info},
    "formatted snapshot info");
_Static_assert(VIEM_FORMATTED_SNAPSHOT_INFO_V1_SIZE ==
    sizeof(ViemFormattedSnapshotInfoV1), "formatted snapshot info size macro");
_Static_assert(offsetof(ViemFormattedSnapshotInfoV1, utf8_length) ==
    {formatted_info_utf8}, "formatted info UTF-8 length offset");
_Static_assert(sizeof(ViemFormattedUtf8RangeV1) == {formatted_range},
    "formatted UTF-8 range");
_Static_assert(VIEM_FORMATTED_UTF8_RANGE_V1_SIZE ==
    sizeof(ViemFormattedUtf8RangeV1), "formatted UTF-8 range size macro");
_Static_assert(offsetof(ViemFormattedUtf8RangeV1, utf8_start) ==
    {formatted_range_start}, "formatted range start offset");
_Static_assert(sizeof(ViemFormattedPointInfoV1) == {formatted_point},
    "formatted point info");
_Static_assert(VIEM_FORMATTED_POINT_INFO_V1_SIZE ==
    sizeof(ViemFormattedPointInfoV1), "formatted point info size macro");
_Static_assert(offsetof(ViemFormattedPointInfoV1, grapheme_column) ==
    {formatted_point_column}, "formatted point column offset");
_Static_assert(sizeof(ViemClipboardTurnEntryV2) == {clipboard_turn_entry},
    "clipboard turn entry");
_Static_assert(VIEM_CLIPBOARD_TURN_ENTRY_V2_SIZE ==
    sizeof(ViemClipboardTurnEntryV2), "clipboard turn entry size macro");
_Static_assert(offsetof(ViemClipboardTurnEntryV2, plain_text) ==
    {clipboard_turn_text}, "clipboard turn text offset");
_Static_assert(sizeof(ViemCommandTurnContextV2) == {command_turn_context},
    "command turn context");
_Static_assert(VIEM_COMMAND_TURN_CONTEXT_V2_SIZE ==
    sizeof(ViemCommandTurnContextV2), "command turn context size macro");
_Static_assert(sizeof(ViemEffectBytesRefV1) == {effect_bytes_ref},
    "effect byte reference");
_Static_assert(sizeof(ViemClipboardWriteV1) == {clipboard_write},
    "clipboard write");
_Static_assert(VIEM_CLIPBOARD_WRITE_V1_SIZE == sizeof(ViemClipboardWriteV1),
    "clipboard write size macro");
_Static_assert(sizeof(ViemExOptionDisplayV1) == {ex_option}, "Ex option");
_Static_assert(VIEM_EX_OPTION_DISPLAY_V1_SIZE == sizeof(ViemExOptionDisplayV1),
    "Ex option size macro");
_Static_assert(sizeof(ViemExMarkV1) == {ex_mark}, "Ex mark");
_Static_assert(VIEM_EX_MARK_V1_SIZE == sizeof(ViemExMarkV1),
    "Ex mark size macro");
_Static_assert(sizeof(ViemExRegisterV1) == {ex_register}, "Ex register");
_Static_assert(VIEM_EX_REGISTER_V1_SIZE == sizeof(ViemExRegisterV1),
    "Ex register size macro");
_Static_assert(sizeof(ViemExJumpV1) == {ex_jump}, "Ex jump");
_Static_assert(VIEM_EX_JUMP_V1_SIZE == sizeof(ViemExJumpV1),
    "Ex jump size macro");
_Static_assert(sizeof(ViemExTextLineV1) == {ex_text_line}, "Ex text line");
_Static_assert(VIEM_EX_TEXT_LINE_V1_SIZE == sizeof(ViemExTextLineV1),
    "Ex text-line size macro");
_Static_assert(sizeof(ViemExFrontendRequestV1) == {ex_request},
    "Ex frontend request");
_Static_assert(VIEM_EX_FRONTEND_REQUEST_V1_SIZE ==
    sizeof(ViemExFrontendRequestV1), "Ex frontend request size macro");
_Static_assert(offsetof(ViemExFrontendRequestV1, first_payload) ==
    {ex_request_payload}, "Ex request payload offset");
_Static_assert(sizeof(ViemEffectBatchInfoV1) == {effect_batch_info},
    "effect batch info");
_Static_assert(VIEM_EFFECT_BATCH_INFO_V1_SIZE == sizeof(ViemEffectBatchInfoV1),
    "effect batch info size macro");
_Static_assert(offsetof(ViemEffectBatchInfoV1, ex_mark_count) ==
    {effect_batch_mark_count}, "effect batch mark count offset");
_Static_assert(sizeof(ViemResolvedTextStyleV1) == {style}, "style");
_Static_assert(offsetof(ViemResolvedTextStyleV1, script_position) == {style_script}, "script position offset");
_Static_assert(_Generic(((ViemResolvedTextStyleV1 *)0)->script_position, uint32_t: 1, default: 0), "script position is an enum integer");
_Static_assert(sizeof(ViemTypographyInfoV1) == {typography_info}, "typography info size");
_Static_assert(offsetof(ViemTypographyInfoV1, script_position) == {typography_script}, "typography script offset");
_Static_assert(offsetof(ViemTypographyInfoV1, background) == {typography_background}, "typography background offset");
_Static_assert(VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION == {script_property}, "script property tag");
_Static_assert(VIEM_STYLE_VALUE_SCRIPT_POSITION == {script_value}, "script value tag");
_Static_assert(VIEM_STYLE_VALUE_PERCENTAGE == {percentage_value}, "percentage value tag");
static ViemStatus (*direct_properties)(ViemCoreHandle, ViemViewId, const ViemDirectStyleEditV1 *, uint64_t, ViemCoreOutcomeV1 *) = viem_core_view_edit_direct_properties;
static ViemStatus (*copy_formatting)(ViemCoreHandle, ViemViewId, const ViemLogicalSelectionIdentityV1 *, ViemStyleSheetInfoV1 *, ViemStylePropertyV1 *, uint64_t, ViemStyleValueItemV1 *, uint64_t, uint8_t *, uint64_t) = viem_core_view_copy_formatting;
static ViemStatus (*typography_export)(ViemCoreHandle, ViemViewId, uint64_t, ViemTypographyInfoV1 *, uint8_t *, uint64_t, ViemOpenTypeFeatureV1 *, uint64_t) = viem_core_view_typography_export;
_Static_assert(sizeof(ViemShapeStyleRunV1) == {style_run}, "style run");
_Static_assert(sizeof(ViemShapedClusterV1) == {cluster}, "cluster");
_Static_assert(sizeof(ViemShapingDiagnosticV1) == {diagnostic}, "diagnostic");
_Static_assert(sizeof(ViemShapeRequestV1) == {request}, "request");
_Static_assert(sizeof(ViemShapeResponseV1) == {response}, "response");
_Static_assert(sizeof(ViemTextMeasurementProviderV1) == {provider}, "provider");
_Static_assert(sizeof(ViemViewOptionsV1) == {view_options}, "view options");
_Static_assert(sizeof(ViemViewportOriginV1) == {viewport_origin}, "viewport origin");
_Static_assert(sizeof(ViemViewportStateV1) == {viewport_state}, "viewport state");
_Static_assert(sizeof(ViemLayoutSnapshotIdentityV1) == {layout_identity},
    "layout identity");
_Static_assert(sizeof(ViemLayoutInsetsV1) == {layout_insets}, "layout insets");
_Static_assert(sizeof(ViemLayoutRectV1) == {layout_rect}, "layout rect");
_Static_assert(sizeof(ViemRgbaV1) == {rgba}, "RGBA");
_Static_assert(sizeof(ViemStyleSheetIdentityV1) == {style_sheet_identity},
    "style-sheet identity");
_Static_assert(VIEM_STYLE_SHEET_IDENTITY_V1_SIZE == sizeof(ViemStyleSheetIdentityV1),
    "style-sheet identity size macro");
_Static_assert(sizeof(ViemStyleStringRefV1) == {style_string_ref},
    "style string reference");
_Static_assert(sizeof(ViemStyleSheetInfoV1) == {style_sheet_info},
    "style-sheet info");
_Static_assert(VIEM_STYLE_SHEET_INFO_V1_SIZE == sizeof(ViemStyleSheetInfoV1),
    "style-sheet info size macro");
_Static_assert(sizeof(ViemStyleValueV1) == {style_value}, "style value");
_Static_assert(VIEM_STYLE_VALUE_V1_SIZE == sizeof(ViemStyleValueV1),
    "style value size macro");
_Static_assert(sizeof(ViemStyleValueItemV1) == {style_value_item},
    "style value item");
_Static_assert(sizeof(ViemStyleDependencyV1) == {style_dependency},
    "style dependency");
_Static_assert(sizeof(ViemStyleDefinitionV1) == {style_definition},
    "style definition");
_Static_assert(sizeof(ViemStylePropertyV1) == {style_property},
    "style property");
_Static_assert(sizeof(ViemStyleEditValueItemV1) == {style_edit_value_item},
    "style edit value item");
_Static_assert(sizeof(ViemStyleEditValueV1) == {style_edit_value},
    "style edit value");
_Static_assert(sizeof(ViemStyleEditV1) == {style_edit}, "style edit");
_Static_assert(VIEM_STYLE_EDIT_V1_SIZE == sizeof(ViemStyleEditV1),
    "style edit size macro");
_Static_assert(sizeof(ViemStyleEditGroupV1) == {style_edit_group},
    "style edit group");
_Static_assert(VIEM_STYLE_EDIT_GROUP_V1_SIZE == sizeof(ViemStyleEditGroupV1),
    "style edit group size macro");
_Static_assert(offsetof(ViemStyleEditGroupV1, token) == {style_edit_group_token},
    "style edit group token offset");
_Static_assert(VIEM_STYLE_EDIT_SET_DISPLAY_NAME == 7u,
    "style display-name operation");
_Static_assert(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME == (1u << 3),
    "style display-name capability");
_Static_assert(sizeof(ViemTextPaintV1) == {text_paint}, "text paint");
_Static_assert(VIEM_TEXT_PAINT_V1_SIZE == sizeof(ViemTextPaintV1),
    "text paint size macro");
_Static_assert(sizeof(ViemLayoutPaintInfoV1) == {layout_paint_info},
    "layout paint info");
_Static_assert(VIEM_LAYOUT_PAINT_INFO_V1_SIZE == sizeof(ViemLayoutPaintInfoV1),
    "layout paint info size macro");
_Static_assert(sizeof(ViemPaintStyleRunV1) == {paint_style_run},
    "paint style run");
_Static_assert(VIEM_PAINT_STYLE_RUN_V1_SIZE == sizeof(ViemPaintStyleRunV1),
    "paint style run size macro");
_Static_assert(sizeof(ViemLayoutSnapshotInfoV1) == {layout_info}, "layout info");
_Static_assert(sizeof(ViemVisualRowV1) == {visual_row}, "visual row");
_Static_assert(sizeof(ViemPositionedClusterV1) == {positioned_cluster},
    "positioned cluster");
_Static_assert(sizeof(ViemPositionedCaretV1) == {positioned_caret},
    "positioned caret");
_Static_assert(sizeof(ViemLayoutCaretRequestV1) == {caret_request},
    "caret request");
_Static_assert(sizeof(ViemLayoutHitTestRequestV1) == {hit_test_request},
    "hit-test request");
_Static_assert(sizeof(ViemLayoutCaretPointV1) == {caret_point}, "caret point");
_Static_assert(sizeof(ViemLayoutCaretGeometryV1) == {caret_geometry},
    "caret geometry");
_Static_assert(sizeof(ViemViewPresentationV1) == {presentation},
    "view presentation");
_Static_assert(sizeof(ViemCommandLineIdentityV1) == {command_line_identity},
    "command-line identity");
_Static_assert(VIEM_COMMAND_LINE_IDENTITY_V1_SIZE == sizeof(ViemCommandLineIdentityV1),
    "command-line identity size macro");
_Static_assert(sizeof(ViemCommandLineInfoV1) == {command_line_info},
    "command-line info");
_Static_assert(VIEM_COMMAND_LINE_INFO_V1_SIZE == sizeof(ViemCommandLineInfoV1),
    "command-line info size macro");
_Static_assert(sizeof(ViemCompletionInfoV1) == {completion_info},
    "completion info");
_Static_assert(VIEM_COMPLETION_INFO_V1_SIZE == sizeof(ViemCompletionInfoV1),
    "completion info size macro");
_Static_assert(offsetof(ViemCompletionInfoV1, selected_index) == {completion_selected},
    "completion signed selection offset");
_Static_assert(offsetof(ViemCompletionInfoV1, anchor_layout) == {completion_anchor_layout},
    "completion anchor identity offset");
_Static_assert(offsetof(ViemCompletionInfoV1, anchor_rect) == {completion_anchor_rect},
    "completion anchor geometry offset");
_Static_assert(sizeof(ViemCompletionItemV1) == {completion_item},
    "completion item");
_Static_assert(VIEM_COMPLETION_ITEM_V1_SIZE == sizeof(ViemCompletionItemV1),
    "completion item size macro");
_Static_assert(VIEM_COMPLETION_ACTIVE == {completion_active}, "completion active flag");
_Static_assert(VIEM_COMPLETION_SEARCHING == {completion_searching}, "completion searching flag");
_Static_assert(VIEM_COMPLETION_TRUNCATED == {completion_truncated}, "completion truncated flag");
_Static_assert(VIEM_COMPLETION_HAS_ANCHOR == {completion_has_anchor}, "completion anchor flag");
_Static_assert(VIEM_COMPLETION_RIGHT_TO_LEFT == {completion_rtl}, "completion RTL flag");
_Static_assert(sizeof(ViemVisualSelectionIdentityV1) == {visual_selection_identity},
    "Visual-selection identity");
_Static_assert(VIEM_VISUAL_SELECTION_IDENTITY_V1_SIZE == sizeof(ViemVisualSelectionIdentityV1),
    "Visual-selection identity size macro");
_Static_assert(sizeof(ViemVisualSelectionInfoV1) == {visual_selection_info},
    "Visual-selection info");
_Static_assert(VIEM_VISUAL_SELECTION_INFO_V1_SIZE == sizeof(ViemVisualSelectionInfoV1),
    "Visual-selection info size macro");
_Static_assert(sizeof(ViemVisualSelectionSegmentV1) == {visual_selection_segment},
    "Visual-selection segment");
_Static_assert(VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE == sizeof(ViemVisualSelectionSegmentV1),
    "Visual-selection segment size macro");
_Static_assert(sizeof(ViemVisualSelectionRectangleV1) == {visual_selection_rectangle},
    "Visual-selection rectangle");
_Static_assert(VIEM_VISUAL_SELECTION_RECTANGLE_V1_SIZE == sizeof(ViemVisualSelectionRectangleV1),
    "Visual-selection rectangle size macro");
_Static_assert(sizeof(ViemPlaceCursorV1) == {place_cursor}, "place cursor");
_Static_assert(sizeof(ViemAssignStyleV1) == {assign_style}, "assign style");
_Static_assert(VIEM_ASSIGN_STYLE_V1_SIZE == sizeof(ViemAssignStyleV1), "assign style size macro");
_Static_assert(sizeof(ViemCreateStyleV1) == {create_style}, "create style");
_Static_assert(VIEM_CREATE_STYLE_V1_SIZE == sizeof(ViemCreateStyleV1), "create style size macro");
_Static_assert(sizeof(ViemDeleteStyleV1) == {delete_style}, "delete style");
_Static_assert(VIEM_DELETE_STYLE_V1_SIZE == sizeof(ViemDeleteStyleV1), "delete style size macro");
_Static_assert(sizeof(ViemSetFileFormatV1) == {set_file_format},
    "set file format");
_Static_assert(VIEM_SET_FILE_FORMAT_V1_SIZE == sizeof(ViemSetFileFormatV1),
    "set file format size macro");
_Static_assert(sizeof(ViemSetIncludeStyleDefinitionsV1) == {set_include_style_definitions},
    "set include style definitions");
_Static_assert(VIEM_SET_INCLUDE_STYLE_DEFINITIONS_V1_SIZE == sizeof(ViemSetIncludeStyleDefinitionsV1),
    "set include style definitions size macro");
_Static_assert(sizeof(ViemMarkSavedV1) == {mark_saved}, "mark saved");
_Static_assert(VIEM_MARK_SAVED_V1_SIZE == sizeof(ViemMarkSavedV1),
    "mark saved size macro");
_Static_assert(sizeof(ViemKeyInputV1) == {key}, "key");
_Static_assert(sizeof(ViemCompositionBeginV1) == {composition_begin}, "composition begin");
_Static_assert(sizeof(ViemCompositionUpdateV1) == {composition_update}, "composition update");
_Static_assert(sizeof(ViemCompositionCommitV1) == {composition_commit}, "composition commit");
_Static_assert(sizeof(ViemCompositionCancelV1) == {composition_cancel}, "composition cancel");
_Static_assert(sizeof(ViemCompositionOverlayIdentityV1) == {composition_overlay_identity},
    "composition overlay identity");
_Static_assert(VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE ==
    sizeof(ViemCompositionOverlayIdentityV1), "composition overlay identity size macro");
_Static_assert(sizeof(ViemCompositionOverlayInfoV1) == {composition_overlay_info},
    "composition overlay info");
_Static_assert(VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE ==
    sizeof(ViemCompositionOverlayInfoV1), "composition overlay info size macro");
_Static_assert(sizeof(ViemCompositionOverlayUtf8RangeV1) == {composition_overlay_range},
    "composition overlay range");
_Static_assert(VIEM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE ==
    sizeof(ViemCompositionOverlayUtf8RangeV1), "composition overlay range size macro");
_Static_assert(sizeof(ViemCoreOutcomeV1) == {outcome}, "outcome");
static void typecheck(void) {{
  ViemShapeRequestV1 request = {{0}};
  request.paragraph_base_direction = VIEM_TEXT_DIRECTION_AUTO;
  ViemStatus (*create_core)(const uint8_t *, uint64_t,
      const ViemDocumentOptions *, ViemCoreHandle *, uint64_t *) = viem_core_create;
  ViemStatus (*document_state)(ViemCoreHandle, ViemDocumentStateV1 *) =
      viem_core_document_state;
  ViemStatus (*formatted_info)(ViemCoreHandle,
      ViemFormattedSnapshotInfoV1 *) = viem_core_formatted_snapshot_info;
  ViemStatus (*copy_formatted_range)(ViemCoreHandle,
      const ViemFormattedUtf8RangeV1 *, uint8_t *, uint64_t, uint64_t *) =
      viem_core_copy_formatted_utf8_range;
  ViemStatus (*map_utf8_to_utf16)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentityV1 *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      viem_core_map_formatted_utf8_to_utf16;
  ViemStatus (*map_utf16_to_utf8)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentityV1 *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      viem_core_map_formatted_utf16_to_utf8;
  ViemStatus (*formatted_point)(ViemCoreHandle,
      const ViemFormattedSnapshotIdentityV1 *, uint64_t,
      ViemFormattedPointInfoV1 *) = viem_core_formatted_point_info;
  ViemStatus (*mark_saved)(ViemCoreHandle, const ViemMarkSavedV1 *) =
      viem_core_mark_saved;
  ViemStatus (*style_sheet_info)(ViemCoreHandle, ViemStyleSheetInfoV1 *) =
      viem_core_style_sheet_info;
  ViemStatus (*copy_style_sheet)(ViemCoreHandle,
      const ViemStyleSheetIdentityV1 *, ViemStyleDefinitionV1 *, uint64_t,
      ViemStylePropertyV1 *, uint64_t, ViemStyleValueItemV1 *, uint64_t,
      ViemStyleDependencyV1 *, uint64_t, uint8_t *, uint64_t,
      ViemStyleSheetInfoV1 *) = viem_core_copy_style_sheet;
  ViemStatus (*add_view)(ViemCoreHandle, const ViemViewOptionsV1 *,
      const ViemTextMeasurementProviderV1 *, ViemViewId *,
      ViemCoreOutcomeV1 *) = viem_core_view_add;
  ViemStatus (*viewport_state)(ViemCoreHandle, ViemViewId,
      ViemViewportStateV1 *) = viem_core_view_viewport_state;
  ViemStatus (*layout_info)(ViemCoreHandle, ViemViewId,
      ViemLayoutSnapshotInfoV1 *) = viem_core_view_layout_snapshot_info;
  ViemStatus (*copy_layout)(ViemCoreHandle, ViemViewId,
      const ViemLayoutSnapshotIdentityV1 *, ViemVisualRowV1 *, uint64_t,
      ViemPositionedClusterV1 *, uint64_t, ViemPositionedCaretV1 *, uint64_t,
      ViemLayoutSnapshotInfoV1 *) = viem_core_view_copy_layout_snapshot;
  ViemStatus (*layout_paint_info)(ViemCoreHandle, ViemViewId,
      ViemLayoutPaintInfoV1 *) = viem_core_view_layout_paint_info;
  ViemStatus (*copy_layout_paint)(ViemCoreHandle, ViemViewId,
      const ViemLayoutSnapshotIdentityV1 *, ViemPaintStyleRunV1 *, uint64_t,
      ViemLayoutPaintInfoV1 *) = viem_core_view_copy_layout_paint;
  ViemStatus (*caret_geometry)(ViemCoreHandle, ViemViewId,
      const ViemLayoutCaretRequestV1 *, ViemLayoutCaretGeometryV1 *) =
      viem_core_view_caret_geometry;
  ViemStatus (*hit_test)(ViemCoreHandle, ViemViewId,
      const ViemLayoutHitTestRequestV1 *, ViemLayoutCaretPointV1 *) =
      viem_core_view_layout_hit_test;
  ViemStatus (*presentation)(ViemCoreHandle, ViemViewId,
      ViemViewPresentationV1 *) = viem_core_view_presentation;
  ViemStatus (*command_line_info)(ViemCoreHandle, ViemViewId,
      ViemCommandLineInfoV1 *) = viem_core_view_command_line_info;
  ViemStatus (*copy_command_line)(ViemCoreHandle, ViemViewId,
      const ViemCommandLineIdentityV1 *, uint8_t *, uint64_t,
      ViemCommandLineInfoV1 *) = viem_core_view_copy_command_line;
  ViemStatus (*completion_info)(ViemCoreHandle, ViemViewId,
      ViemCompletionInfoV1 *) = viem_core_view_completion_info;
  ViemStatus (*copy_completion_items)(ViemCoreHandle, ViemViewId,
      const ViemCompletionInfoV1 *, ViemCompletionItemV1 *, uint64_t,
      uint64_t *) = viem_core_view_copy_completion_items;
  ViemStatus (*copy_completion_utf8)(ViemCoreHandle, ViemViewId,
      const ViemCompletionInfoV1 *, uint8_t *, uint64_t,
      uint64_t *) = viem_core_view_copy_completion_utf8;
  ViemStatus (*poll_completion)(ViemCoreHandle, ViemViewId,
      uint8_t *) = viem_core_view_poll_completion;
  ViemStatus (*accept_completion)(ViemCoreHandle, ViemViewId,
      uint8_t *) = viem_core_view_accept_completion;
  ViemStatus (*visual_selection_info)(ViemCoreHandle, ViemViewId,
      ViemVisualSelectionInfoV1 *) = viem_core_view_visual_selection_info;
  ViemStatus (*copy_visual_selection)(ViemCoreHandle, ViemViewId,
      const ViemVisualSelectionIdentityV1 *,
      ViemVisualSelectionSegmentV1 *, uint64_t,
      ViemVisualSelectionRectangleV1 *, uint64_t,
      ViemVisualSelectionInfoV1 *) = viem_core_view_copy_visual_selection;
  ViemStatus (*use_selection_for_find)(ViemCoreHandle, ViemViewId,
      const ViemVisualSelectionIdentityV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_use_selection_for_find;
  ViemStatus (*reveal_selection)(ViemCoreHandle, ViemViewId,
      ViemCoreOutcomeV1 *) = viem_core_view_reveal_selection;
  ViemStatus (*set_viewport_origin)(ViemCoreHandle, ViemViewId,
      const ViemViewportOriginV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_set_viewport_origin;
  ViemStatus (*set_scale)(ViemCoreHandle, ViemViewId, float,
      ViemCoreOutcomeV1 *) = viem_core_view_set_scale;
  ViemStatus (*set_linebreak)(ViemCoreHandle, ViemViewId, uint32_t,
      ViemCoreOutcomeV1 *) = viem_core_view_set_linebreak;
  ViemStatus (*set_file_format)(ViemCoreHandle, ViemViewId,
      const ViemSetFileFormatV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_set_file_format;
  ViemStatus (*set_include_style_definitions)(ViemCoreHandle, ViemViewId,
      const ViemSetIncludeStyleDefinitionsV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_set_include_style_definitions;
  ViemStatus (*edit_style)(ViemCoreHandle, ViemViewId,
      const ViemStyleEditV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_edit_style;
  ViemStatus (*assign_style)(ViemCoreHandle, ViemViewId,
      const ViemAssignStyleV1 *, ViemCoreOutcomeV1 *) = viem_core_view_assign_style;
  ViemStatus (*create_style)(ViemCoreHandle, ViemViewId,
      const ViemCreateStyleV1 *, ViemCoreOutcomeV1 *) = viem_core_view_create_style;
  ViemStatus (*delete_style)(ViemCoreHandle, ViemViewId,
      const ViemDeleteStyleV1 *, ViemCoreOutcomeV1 *) = viem_core_view_delete_style;
  ViemStatus (*begin_style_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleSheetIdentityV1 *, ViemStyleEditGroupV1 *) =
      viem_core_view_begin_style_edit_group;
  ViemStatus (*edit_style_in_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleEditGroupV1 *, const ViemStyleEditV1 *,
      ViemCoreOutcomeV1 *) = viem_core_view_edit_style_in_group;
  ViemStatus (*end_style_group)(ViemCoreHandle, ViemViewId,
      const ViemStyleEditGroupV1 *) = viem_core_view_end_style_edit_group;
  ViemStatus (*send_key_with_host_context)(ViemCoreHandle, ViemViewId,
      const ViemKeyInputV1 *, const ViemCommandTurnContextV2 *,
      ViemCoreOutcomeV1 *, ViemEffectBatchHandle *) =
      viem_core_view_send_key_with_host_context_v2;
  ViemStatus (*send_text_with_host_context)(ViemCoreHandle, ViemViewId,
      const uint8_t *, uint64_t, const ViemCommandTurnContextV2 *,
      ViemCoreOutcomeV1 *, ViemEffectBatchHandle *) =
      viem_core_view_send_text_with_host_context_v2;
  ViemStatus (*effect_info)(ViemEffectBatchHandle,
      ViemEffectBatchInfoV1 *) = viem_effect_batch_info;
  ViemStatus (*effect_copy)(ViemEffectBatchHandle,
      ViemClipboardWriteV1 *, uint64_t,
      ViemExFrontendRequestV1 *, uint64_t,
      ViemExOptionDisplayV1 *, uint64_t,
      ViemExMarkV1 *, uint64_t,
      ViemExRegisterV1 *, uint64_t,
      ViemExJumpV1 *, uint64_t,
      ViemExTextLineV1 *, uint64_t,
      uint32_t *, uint64_t, uint64_t *, uint64_t,
      uint8_t *, uint64_t, ViemEffectBatchInfoV1 *) =
      viem_effect_batch_copy;
  ViemStatus (*effect_release)(ViemEffectBatchHandle) =
      viem_effect_batch_release;
  ViemStatus (*place_cursor)(ViemCoreHandle, ViemViewId,
      const ViemPlaceCursorV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_place_cursor;
  ViemStatus (*undo)(ViemCoreHandle, ViemViewId, ViemCoreOutcomeV1 *) =
      viem_core_view_undo;
  ViemStatus (*redo)(ViemCoreHandle, ViemViewId, ViemCoreOutcomeV1 *) =
      viem_core_view_redo;
  ViemStatus (*composition_begin)(ViemCoreHandle, ViemViewId,
      const ViemCompositionBeginV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_composition_begin;
  ViemStatus (*composition_update)(ViemCoreHandle, ViemViewId,
      const ViemCompositionUpdateV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_composition_update;
  ViemStatus (*composition_overlay_info)(ViemCoreHandle, ViemViewId,
      ViemCompositionOverlayInfoV1 *) =
      viem_core_view_composition_overlay_info;
  ViemStatus (*copy_composition_range)(ViemCoreHandle, ViemViewId,
      const ViemCompositionOverlayUtf8RangeV1 *, uint8_t *, uint64_t,
      uint64_t *) = viem_core_view_copy_composition_utf8_range;
  ViemStatus (*composition_commit)(ViemCoreHandle, ViemViewId,
      const ViemCompositionCommitV1 *, ViemCoreOutcomeV1 *) =
      viem_core_view_composition_commit;
  ViemStatus (*composition_cancel)(ViemCoreHandle, ViemViewId,
      const ViemCompositionCancelV1 *, ViemCoreOutcomeV1 *) =
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
  (void)set_linebreak; (void)set_file_format;
  (void)set_include_style_definitions;
  (void)edit_style; (void)begin_style_group; (void)edit_style_in_group;
  (void)assign_style;
  (void)create_style; (void)delete_style;
  (void)end_style_group;

  (void)send_key_with_host_context;
  (void)send_text_with_host_context; (void)effect_info;
  (void)effect_copy; (void)effect_release;
  (void)place_cursor; (void)undo; (void)redo;
  (void)composition_begin; (void)composition_update;
  (void)composition_overlay_info; (void)copy_composition_range;
  (void)composition_commit; (void)composition_cancel;
}}
"#,
        abi = VIEM_CORE_ABI_VERSION,
        selection_character = VIEM_MODE_SELECTION_CHARACTER,
        selection_line = VIEM_MODE_SELECTION_LINE,
        selection_block = VIEM_MODE_SELECTION_BLOCK,
        autoselect = VIEM_EX_OPTION_AUTOSELECT,
        set_format_size = std::mem::size_of::<ViemSetFormatV1>(),
        set_format_operation = std::mem::offset_of!(ViemSetFormatV1, operation),
        set_format_document = std::mem::offset_of!(ViemSetFormatV1, document_id),
        reinterpret = VIEM_FORMAT_OPERATION_REINTERPRET,
        convert = VIEM_FORMAT_OPERATION_CONVERT,
        direct_style_edit = std::mem::size_of::<ViemDirectStyleEditV1>(),
        direct_style_align = std::mem::align_of::<ViemDirectStyleEditV1>(),
        direct_style_selection = std::mem::offset_of!(ViemDirectStyleEditV1, expected_selection),
        direct_style_value = std::mem::offset_of!(ViemDirectStyleEditV1, value),
        document_options = std::mem::size_of::<ViemDocumentOptions>(),
        document_state = std::mem::size_of::<ViemDocumentStateV1>(),
        formatted_identity = std::mem::size_of::<ViemFormattedSnapshotIdentityV1>(),
        formatted_identity_document =
            std::mem::offset_of!(ViemFormattedSnapshotIdentityV1, document_id),
        formatted_info = std::mem::size_of::<ViemFormattedSnapshotInfoV1>(),
        formatted_info_utf8 = std::mem::offset_of!(ViemFormattedSnapshotInfoV1, utf8_length),
        formatted_range = std::mem::size_of::<ViemFormattedUtf8RangeV1>(),
        formatted_range_start = std::mem::offset_of!(ViemFormattedUtf8RangeV1, utf8_start),
        formatted_point = std::mem::size_of::<ViemFormattedPointInfoV1>(),
        formatted_point_column = std::mem::offset_of!(ViemFormattedPointInfoV1, grapheme_column),
        clipboard_turn_entry = std::mem::size_of::<ViemClipboardTurnEntryV2>(),
        clipboard_turn_text = std::mem::offset_of!(ViemClipboardTurnEntryV2, plain_text),
        command_turn_context = std::mem::size_of::<ViemCommandTurnContextV2>(),
        effect_bytes_ref = std::mem::size_of::<ViemEffectBytesRefV1>(),
        clipboard_write = std::mem::size_of::<ViemClipboardWriteV1>(),
        ex_option = std::mem::size_of::<ViemExOptionDisplayV1>(),
        ex_mark = std::mem::size_of::<ViemExMarkV1>(),
        ex_register = std::mem::size_of::<ViemExRegisterV1>(),
        ex_jump = std::mem::size_of::<ViemExJumpV1>(),
        ex_text_line = std::mem::size_of::<ViemExTextLineV1>(),
        ex_request = std::mem::size_of::<ViemExFrontendRequestV1>(),
        ex_request_payload = std::mem::offset_of!(ViemExFrontendRequestV1, first_payload),
        effect_batch_info = std::mem::size_of::<ViemEffectBatchInfoV1>(),
        effect_batch_mark_count = std::mem::offset_of!(ViemEffectBatchInfoV1, ex_mark_count),
        style = std::mem::size_of::<ViemResolvedTextStyleV1>(),
        style_script = std::mem::offset_of!(ViemResolvedTextStyleV1, script_position),
        typography_info = std::mem::size_of::<ViemTypographyInfoV1>(),
        typography_script = std::mem::offset_of!(ViemTypographyInfoV1, script_position),
        typography_background = std::mem::offset_of!(ViemTypographyInfoV1, background),
        script_property = VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION,
        script_value = VIEM_STYLE_VALUE_SCRIPT_POSITION,
        percentage_value = VIEM_STYLE_VALUE_PERCENTAGE,
        style_run = std::mem::size_of::<ViemShapeStyleRunV1>(),
        cluster = std::mem::size_of::<ViemShapedClusterV1>(),
        diagnostic = std::mem::size_of::<ViemShapingDiagnosticV1>(),
        request = std::mem::size_of::<ViemShapeRequestV1>(),
        response = std::mem::size_of::<ViemShapeResponseV1>(),
        provider = std::mem::size_of::<ViemTextMeasurementProviderV1>(),
        view_options = std::mem::size_of::<ViemViewOptionsV1>(),
        viewport_origin = std::mem::size_of::<ViemViewportOriginV1>(),
        viewport_state = std::mem::size_of::<ViemViewportStateV1>(),
        layout_identity = std::mem::size_of::<ViemLayoutSnapshotIdentityV1>(),
        layout_insets = std::mem::size_of::<ViemLayoutInsetsV1>(),
        layout_rect = std::mem::size_of::<ViemLayoutRectV1>(),
        rgba = std::mem::size_of::<ViemRgbaV1>(),
        style_sheet_identity = std::mem::size_of::<ViemStyleSheetIdentityV1>(),
        style_string_ref = std::mem::size_of::<ViemStyleStringRefV1>(),
        style_sheet_info = std::mem::size_of::<ViemStyleSheetInfoV1>(),
        style_value = std::mem::size_of::<ViemStyleValueV1>(),
        style_value_item = std::mem::size_of::<ViemStyleValueItemV1>(),
        style_dependency = std::mem::size_of::<ViemStyleDependencyV1>(),
        style_definition = std::mem::size_of::<ViemStyleDefinitionV1>(),
        style_property = std::mem::size_of::<ViemStylePropertyV1>(),
        style_edit_value_item = std::mem::size_of::<ViemStyleEditValueItemV1>(),
        style_edit_value = std::mem::size_of::<ViemStyleEditValueV1>(),
        style_edit = std::mem::size_of::<ViemStyleEditV1>(),
        style_edit_group = std::mem::size_of::<ViemStyleEditGroupV1>(),
        style_edit_group_token = std::mem::offset_of!(ViemStyleEditGroupV1, token),
        text_paint = std::mem::size_of::<ViemTextPaintV1>(),
        layout_paint_info = std::mem::size_of::<ViemLayoutPaintInfoV1>(),
        paint_style_run = std::mem::size_of::<ViemPaintStyleRunV1>(),
        layout_info = std::mem::size_of::<ViemLayoutSnapshotInfoV1>(),
        visual_row = std::mem::size_of::<ViemVisualRowV1>(),
        positioned_cluster = std::mem::size_of::<ViemPositionedClusterV1>(),
        positioned_caret = std::mem::size_of::<ViemPositionedCaretV1>(),
        caret_request = std::mem::size_of::<ViemLayoutCaretRequestV1>(),
        hit_test_request = std::mem::size_of::<ViemLayoutHitTestRequestV1>(),
        caret_point = std::mem::size_of::<ViemLayoutCaretPointV1>(),
        caret_geometry = std::mem::size_of::<ViemLayoutCaretGeometryV1>(),
        presentation = std::mem::size_of::<ViemViewPresentationV1>(),
        command_line_identity = std::mem::size_of::<ViemCommandLineIdentityV1>(),
        command_line_info = std::mem::size_of::<ViemCommandLineInfoV1>(),
        completion_info = std::mem::size_of::<ViemCompletionInfoV1>(),
        completion_selected = std::mem::offset_of!(ViemCompletionInfoV1, selected_index),
        completion_anchor_layout = std::mem::offset_of!(ViemCompletionInfoV1, anchor_layout),
        completion_anchor_rect = std::mem::offset_of!(ViemCompletionInfoV1, anchor_rect),
        completion_item = std::mem::size_of::<ViemCompletionItemV1>(),
        completion_active = VIEM_COMPLETION_ACTIVE,
        completion_searching = VIEM_COMPLETION_SEARCHING,
        completion_truncated = VIEM_COMPLETION_TRUNCATED,
        completion_has_anchor = VIEM_COMPLETION_HAS_ANCHOR,
        completion_rtl = VIEM_COMPLETION_RIGHT_TO_LEFT,
        visual_selection_identity = std::mem::size_of::<ViemVisualSelectionIdentityV1>(),
        visual_selection_info = std::mem::size_of::<ViemVisualSelectionInfoV1>(),
        visual_selection_segment = std::mem::size_of::<ViemVisualSelectionSegmentV1>(),
        visual_selection_rectangle = std::mem::size_of::<ViemVisualSelectionRectangleV1>(),
        place_cursor = std::mem::size_of::<ViemPlaceCursorV1>(),
        assign_style = std::mem::size_of::<ViemAssignStyleV1>(),
        create_style = std::mem::size_of::<ViemCreateStyleV1>(),
        delete_style = std::mem::size_of::<ViemDeleteStyleV1>(),
        set_file_format = std::mem::size_of::<ViemSetFileFormatV1>(),
        set_include_style_definitions = std::mem::size_of::<ViemSetIncludeStyleDefinitionsV1>(),
        mark_saved = std::mem::size_of::<ViemMarkSavedV1>(),
        key = std::mem::size_of::<ViemKeyInputV1>(),
        composition_begin = std::mem::size_of::<ViemCompositionBeginV1>(),
        composition_update = std::mem::size_of::<ViemCompositionUpdateV1>(),
        composition_commit = std::mem::size_of::<ViemCompositionCommitV1>(),
        composition_cancel = std::mem::size_of::<ViemCompositionCancelV1>(),
        composition_overlay_identity = std::mem::size_of::<ViemCompositionOverlayIdentityV1>(),
        composition_overlay_info = std::mem::size_of::<ViemCompositionOverlayInfoV1>(),
        composition_overlay_range = std::mem::size_of::<ViemCompositionOverlayUtf8RangeV1>(),
        outcome = std::mem::size_of::<ViemCoreOutcomeV1>(),
    )
}
