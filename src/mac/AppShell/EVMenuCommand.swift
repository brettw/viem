import AppKit

/// Stable tags used to route native actions to the same core intentions as keys.
public enum EVMenuCommand: Int, CaseIterable, Sendable {
    case save = 100
    case saveAs
    case duplicateDocument
    case renameDocument
    case moveDocument
    case revertLastSaved
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

    case convertToText = 130
    case convertToMarkdown
    case convertToHTML
    case reinterpretAsText
    case reinterpretAsMarkdown
    case reinterpretAsHTML

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

    case showFonts = 300
    case bold
    case italic
    case underline
    case strikethrough
    case bigger
    case smaller
    case defaultLigatures
    case allLigatures
    case noLigatures
    case superscript = 312
    case subscriptBaseline
    case raiseBaseline
    case lowerBaseline
    case openTypeFeatures
    case showColors
    case textColor
    case highlightColor
    case baseCharacterStyle
    case editCharacterStyles
    case baseParagraphStyle
    case editParagraphStyles
    case baseDocumentStyle
    case editDocumentStyles
    case saveDefaultStyle
    case alignStart
    case alignCenter
    case alignEnd
    case directionAutomatic
    case directionLeftToRight
    case directionRightToLeft
    case increaseIndent
    case decreaseIndent
    case paragraphSpacing
    case lineSpacingNormal
    case lineSpacingSingle
    case lineSpacingOneAndHalf
    case lineSpacingDouble
    case lineSpacingCustom
    case copyStyle
    case pasteStyle
    case clearDirectCharacterFormatting
    case clearDirectParagraphFormatting
    case clearAllDirectFormatting
    case bulletedList
    case numberedList
    case removeList
    case includeStyleDefinitionsInFile
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

    public var formatChange: (format: EVSourceFormat, operation: EVFormatOperation)? {
        switch self {
        case .convertToText: (.plainText, .convert)
        case .convertToMarkdown: (.markdown, .convert)
        case .convertToHTML: (.html, .convert)
        case .reinterpretAsText: (.plainText, .reinterpret)
        case .reinterpretAsMarkdown: (.markdown, .reinterpret)
        case .reinterpretAsHTML: (.html, .reinterpret)
        default: nil
        }
    }

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
}

@MainActor
@objc public protocol EVEditorCommandRouting {
    func performEditorMenuCommand(_ sender: Any?)
}

/// The three style roles exposed by the native Format menu. Stable style IDs,
/// rather than menu positions, identify the selected definition.
public enum EVStyleMenuRole: UInt32, CaseIterable, Sendable {
    case character = 1
    case paragraph = 2
    case document = 3
}

/// A single definition in the current core-owned style catalogue.
///
/// `presentation` describes assignment at the current selection. It can be
/// disabled when the active format adapter cannot translate that semantic
/// edit, even though the definition remains available in Edit Styles.
public struct EVStyleMenuEntry: Equatable, Sendable {
    public let role: EVStyleMenuRole
    public let stableID: String
    public let displayName: String
    public let isBase: Bool
    public let presentation: EVMenuItemPresentation

    public init(
        role: EVStyleMenuRole,
        stableID: String,
        displayName: String,
        isBase: Bool,
        presentation: EVMenuItemPresentation
    ) {
        self.role = role
        self.stableID = stableID
        self.displayName = displayName
        self.isBase = isBase
        self.presentation = presentation
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

    public init(
        kind: EVStyleMenuActionKind,
        role: EVStyleMenuRole,
        stableID: String,
        documentID: UInt64,
        documentRevision: UInt64,
        styleSheetRevision: UInt64
    ) {
        self.kind = kind
        self.role = role
        self.stableID = stableID
        self.documentID = documentID
        self.documentRevision = documentRevision
        self.styleSheetRevision = styleSheetRevision
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
