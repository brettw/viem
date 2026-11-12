import AppKit
import CViemCore
import ViemAppShell

/// Core's availability and state for the native formatting actions. Menus,
/// the toolbar and accelerators present it without deriving their own.
struct EVFormattingActions {
  private let info: ViemFormattingActions

  init(_ info: ViemFormattingActions) { self.info = info }

  static func bit(_ action: UInt32) -> UInt64 { 1 << UInt64(action) }

  /// The actions that decide which style assignments menus offer.
  static let styleAssignments = bit(UInt32(VIEM_FORMATTING_PARAGRAPH_STYLES))
    | bit(UInt32(VIEM_FORMATTING_CHARACTER_STYLES))

  /// The action for a `VIEM_ALERT_*` kind.
  static func alert(_ kind: UInt32) -> UInt32 {
    UInt32(VIEM_FORMATTING_ALERT_NOTE) + kind - UInt32(VIEM_ALERT_NOTE)
  }

  func presentation(_ action: UInt32) -> EVMenuItemPresentation {
    let bit = Self.bit(action)
    return EVMenuItemPresentation(isEnabled: info.enabled & bit != 0,
      state: info.on & bit != 0 ? .on : info.mixed & bit != 0 ? .mixed : .off)
  }

  func isEnabled(_ action: UInt32) -> Bool { info.enabled & Self.bit(action) != 0 }
}

extension EVCoreViewSession {
  /// Only `requested` actions (bits `1 << action`) are evaluated.
  func formattingActions(_ requested: UInt64 = .max) throws -> EVFormattingActions {
    var info = ViemFormattingActions()
    let status = viem_core_view_formatting_actions(document.core, viewID, requested, &info)
    guard status == UInt32(VIEM_STATUS_OK) else {
      throw EVCoreFrontendError.core(operation: "Read formatting actions", status: status)
    }
    return EVFormattingActions(info)
  }
}

extension EVMenuCommand {
  /// The core formatting action this command presents. Insert Link opens the
  /// link editor, unlike the toolbar's link toggle, so it has none.
  var formattingAction: UInt32? {
    switch self {
    case .bold: UInt32(VIEM_FORMATTING_BOLD)
    case .italic: UInt32(VIEM_FORMATTING_ITALIC)
    case .underline: UInt32(VIEM_FORMATTING_UNDERLINE)
    case .strikethrough: UInt32(VIEM_FORMATTING_STRIKETHROUGH)
    case .superscript: UInt32(VIEM_FORMATTING_SUPERSCRIPT)
    case .subscript: UInt32(VIEM_FORMATTING_SUBSCRIPT)
    case .characterCode: UInt32(VIEM_FORMATTING_CODE)
    case .codeBlock: UInt32(VIEM_FORMATTING_CODE_BLOCK)
    case .blockQuote: UInt32(VIEM_FORMATTING_BLOCK_QUOTE)
    case .alertNote, .alertTip, .alertImportant, .alertWarning, .alertCaution:
      alertKind.map { EVFormattingActions.alert($0) }
    case .bulletedList: UInt32(VIEM_FORMATTING_BULLETED_LIST)
    case .numberedList: UInt32(VIEM_FORMATTING_NUMBERED_LIST)
    case .taskList: UInt32(VIEM_FORMATTING_TASK_LIST)
    case .removeList: UInt32(VIEM_FORMATTING_REMOVE_LIST)
    case .increaseIndent: UInt32(VIEM_FORMATTING_INDENT)
    case .decreaseIndent: UInt32(VIEM_FORMATTING_UNINDENT)
    case .insertListParagraph: UInt32(VIEM_FORMATTING_LIST_PARAGRAPH)
    case .heading0, .heading1, .heading2, .heading3, .heading4, .heading5, .heading6:
      UInt32(VIEM_FORMATTING_HEADING_0) + UInt32(rawValue - EVMenuCommand.heading0.rawValue)
    case .insertReference: UInt32(VIEM_FORMATTING_REFERENCE)
    case .insertImage: UInt32(VIEM_FORMATTING_IMAGE)
    case .insertTable: UInt32(VIEM_FORMATTING_TABLE)
    default: nil
    }
  }
}

@MainActor
extension EVEditorSurfaceController {
  /// One action, evaluated alone so validating a menu item stays cheap.
  func formattingPresentation(_ action: UInt32) -> EVMenuItemPresentation {
    guard let actions = try? session?.formattingActions(EVFormattingActions.bit(action)) else { return .disabled }
    return actions.presentation(action)
  }
}
