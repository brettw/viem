import AppKit

/// Stable tags used to route native actions to the same core intentions as keys.
public enum EVMenuCommand: Int, CaseIterable, Sendable {
    case save = 100
    case saveAs
    case duplicateDocument
    case exportHTML
    case revertLastSaved = 105
    case browseVersions
    case lineEndingUnix = 109
    case lineEndingWindows
    case lineEndingClassicMac
    case pageSetup
    case printDocument

    case encodingUTF8 = 120
    case encodingLatin1
    case encodingUTF16LE
    case encodingUTF16BE

    case undo = 200
    case redo
    case cut
    case copy
    case paste
    case pasteAndMatchStyle
    case delete
    case selectAll
    case selectWord
    case selectSentence
    case selectParagraph
    case selectHardLine
    case selectVisualRow
    case find
    case findAndReplace
    case findNext
    case findPrevious
    case useSelectionForFind
    case jumpToSelection
    case makeUppercase
    case makeLowercase
    case toggleCase

    case copySource = 250

    case bold = 301
    case italic
    case underline = 303
    case strikethrough = 304
    case superscript
    case `subscript`
    case characterCode
    case insertLink
    case defaultParagraphStyle = 320
    case characterStyles
    case baseParagraphStyle
    case paragraphStyles
    case editStyles
    case increaseIndent = 331
    case decreaseIndent
    case bulletedList = 344
    case numberedList
    case removeList
    case reloadStyleSheet
    case heading0 = 370
    case heading1
    case heading2
    case heading3
    case heading4
    case heading5
    case heading6

    case wordWrap = 400
    case showInvisibleCharacters = 402
    case zoomIn
    case zoomOut
    case actualSize
    case newWindowForDocument
    case flowParagraphs

    /// Standard text actions must reach native field editors first. The custom
    /// document view implements the same selectors using core intentions.
    public var nativeEditAction: Selector? {
        let name: String
        switch self {
        case .undo: name = "undo:"
        case .redo: name = "redo:"
        case .cut: name = "cut:"
        case .copy: name = "copy:"
        case .copySource: name = "copySource:"
        case .paste: name = "paste:"
        case .pasteAndMatchStyle: name = "pasteAsPlainText:"
        case .delete: name = "delete:"
        case .selectAll: name = "selectAll:"
        default: return nil
        }
        return NSSelectorFromString(name)
    }

    public static func nativeEditCommand(for action: Selector?) -> EVMenuCommand? {
        guard let action else { return nil }
        return [.undo, .redo, .cut, .copy, .copySource, .paste, .pasteAndMatchStyle, .delete, .selectAll]
            .first { $0.nativeEditAction == action }
    }

    /// One accelerator definition supplies native menus, editor dispatch and
    /// toolbar hover labels, including actions available with the toolbar hidden.
    public var formattingShortcut: EVFormattingShortcut? {
        switch self {
        case .bold: .init(key: "b", modifiers: [.command], label: "⌘B")
        case .italic: .init(key: "i", modifiers: [.command], label: "⌘I")
        case .underline: .init(key: "u", modifiers: [.command], label: "⌘U")
        case .characterCode: .init(key: "c", modifiers: [.option, .command], label: "⌥⌘C")
        case .superscript: .init(key: "+", modifiers: [.control, .command], label: "⌃⌘+")
        case .subscript: .init(key: "-", modifiers: [.control, .command], label: "⌃⌘−")
        case .insertLink: .init(key: "k", modifiers: [.command], label: "⌘K")
        case .bulletedList: .init(key: "8", modifiers: [.shift, .command], label: "⇧⌘8")
        case .numberedList: .init(key: "7", modifiers: [.shift, .command], label: "⇧⌘7")
        default: nil
        }
    }

    public static func formattingCommand(for event: NSEvent) -> EVMenuCommand? {
        allCases.first { $0.formattingShortcut?.matches(event) == true }
    }
}

public struct EVFormattingShortcut {
    public let key: String
    public let modifiers: NSEvent.ModifierFlags
    public let label: String

    public func matches(_ event: NSEvent) -> Bool {
        let actualModifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        let actualKey = event.charactersIgnoringModifiers?.lowercased()
        if key == "+" {
            // A layout with a dedicated Plus key needs no Shift. A US layout
            // supplies Plus from Shift-Equals; AppKit reports either spelling.
            return (actualKey == "+" && (actualModifiers == modifiers || actualModifiers == modifiers.union(.shift)))
                || (actualKey == "=" && actualModifiers == modifiers.union(.shift))
        }
        if modifiers.contains(.shift), key == "7" || key == "8" {
            // charactersIgnoringModifiers retains Shift. Re-translate the
            // numeric key through the active layout before using the supplied
            // characters as a fallback for synthetic events.
            let unshifted = event.characters(byApplyingModifiers:
                event.modifierFlags.subtracting([.command, .control, .option, .shift]))
            if let unshifted, !unshifted.isEmpty {
                return actualModifiers == modifiers && unshifted == key
            }
            let shifted = key == "7" ? "&" : "*"
            return actualModifiers == modifiers && (actualKey == key || actualKey == shifted)
        }
        return actualModifiers == modifiers && actualKey == key
    }
}

@MainActor
@objc public protocol EVEditorCommandRouting {
    func performEditorMenuCommand(_ sender: Any?)
}

/// Formatting accelerators belong only to the focused document editor, so a
/// native field editor cannot fall through to document formatting on its pane.
@MainActor
@objc public protocol EVFormattingCommandRouting {
    func performEditorFormattingCommand(_ sender: Any?)
}

/// The two style roles exposed by the native Style menu. Stable style IDs,
/// rather than menu positions, identify the selected definition.
public enum EVStyleMenuRole: UInt32, CaseIterable, Sendable {
    case character = 1
    case paragraph = 2
}

/// A single definition in the current core-owned style catalogue.
///
/// `presentation` describes the action at the current selection. It can be
/// disabled when the active format adapter cannot translate that semantic
/// edit, even though the definition remains available in Edit Styles.
public struct EVStyleMenuEntry: Equatable, Sendable {
    public let role: EVStyleMenuRole
    public let stableID: String
    public let displayName: String
    public let isBase: Bool
    public let presentation: EVMenuItemPresentation
    public let actionKind: EVStyleMenuActionKind
    public let syntaxName: String?

    public init(
        role: EVStyleMenuRole,
        stableID: String,
        displayName: String,
        isBase: Bool,
        presentation: EVMenuItemPresentation,
        actionKind: EVStyleMenuActionKind = .assign,
        syntaxName: String? = nil
    ) {
        self.role = role
        self.stableID = stableID
        self.displayName = displayName
        self.isBase = isBase
        self.presentation = presentation
        self.actionKind = actionKind
        self.syntaxName = syntaxName
    }
}

/// One exact view of the core style catalogue. The identity is useful to
/// providers which can apply an assignment and must reject a stale menu turn.
public struct EVStyleMenuCatalogue: Equatable, Sendable {
    public let documentID: UInt64
    public let documentRevision: UInt64
    public let styleSheetRevision: UInt64
    public let entries: [EVStyleMenuEntry]
    public let canEditStyles: Bool

    public init(
        documentID: UInt64,
        documentRevision: UInt64,
        styleSheetRevision: UInt64,
        entries: [EVStyleMenuEntry],
        canEditStyles: Bool
    ) {
        self.documentID = documentID
        self.documentRevision = documentRevision
        self.styleSheetRevision = styleSheetRevision
        self.entries = entries
        self.canEditStyles = canEditStyles
    }
}

public enum EVStyleMenuActionKind: UInt32, Sendable {
    case assign = 1
    case edit = 2
    /// Explicit creation of an unresolved automatic syntax name. No definition
    /// or stable ID exists until the user invokes this action.
    case defineSyntax = 3
    /// Open the style currently selected in this namespace at invocation time.
    case editCurrent = 4
}

/// Typed payload carried by style menu items. Keeping both the role and stable
/// ID here prevents duplicate display names, sorting, or a refresh from
/// changing the identity of the requested style.
public final class EVStyleMenuAction: NSObject, @unchecked Sendable {
    public let kind: EVStyleMenuActionKind
    public let role: EVStyleMenuRole
    public let stableID: String
    public let documentID: UInt64
    public let documentRevision: UInt64
    public let styleSheetRevision: UInt64
    public let syntaxName: String?
    /// Definition rows show their active style state; generic editor actions
    /// do not. This controls presentation only, never action authorization.
    public var marksCurrentStyle: Bool { kind != .editCurrent }

    public init(
        kind: EVStyleMenuActionKind,
        role: EVStyleMenuRole,
        stableID: String,
        documentID: UInt64,
        documentRevision: UInt64,
        styleSheetRevision: UInt64,
        syntaxName: String? = nil
    ) {
        self.kind = kind
        self.role = role
        self.stableID = stableID
        self.documentID = documentID
        self.documentRevision = documentRevision
        self.styleSheetRevision = styleSheetRevision
        self.syntaxName = syntaxName
    }
}

/// Optional capability implemented by a concrete editor surface. AppShell
/// asks for a new snapshot whenever a style submenu opens; it never owns or
/// mutates a private style catalogue.
@MainActor
public protocol EVStyleMenuProviding: AnyObject {
    func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue?
}

/// Style actions stay on AppKit's responder chain just like the other editor
/// actions. The first responder unwraps the typed payload and dispatches the
/// corresponding core-backed semantic intention.
@MainActor
@objc public protocol EVStyleMenuActionRouting: AnyObject {
    func performEditorStyleMenuAction(_ sender: Any?)
}
