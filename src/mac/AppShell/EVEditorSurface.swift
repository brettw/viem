import AppKit
import Foundation

public enum EVStatusBarOption: Equatable, Sendable {
    case format(EVSourceFormat)
    case encoding(UInt32)
    case lineEnding(UInt32)
}

/// Presentation-only values shown by the window's status bar.
public struct EVStatusBarState: Equatable, Sendable {
    public var mode: String
    public var message: String
    public var location: String
    public var encoding: String
    public var lineEnding: String
    public var format: String

    public init(
        mode: String = "NORMAL",
        message: String = "",
        location: String = "Ln 1, Col 1",
        encoding: String = "UTF-8",
        lineEnding: String = "LF",
        format: String = "Plain Text"
    ) {
        self.mode = mode
        self.message = message
        self.location = location
        self.encoding = encoding
        self.lineEnding = lineEnding
        self.format = format
    }
}

/// The current validation result for a core-backed menu command.
public struct EVMenuItemPresentation: Equatable, Sendable {
    public var isEnabled: Bool
    public var state: NSControl.StateValue
    public var title: String?

    public init(
        isEnabled: Bool,
        state: NSControl.StateValue = .off,
        title: String? = nil
    ) {
        self.isEnabled = isEnabled
        self.state = state
        self.title = title
    }

    public static let disabled = EVMenuItemPresentation(isEnabled: false)
    public static let enabled = EVMenuItemPresentation(isEnabled: true)
}

/// Immutable source bytes captured for one native NSDocument save.
///
/// The identifiers are opaque to AppKit. They let the backend acknowledge
/// exactly the snapshot which was written instead of whichever revision is
/// current by the time native file I/O completes.
public struct EVDocumentSaveSnapshot: Equatable, Sendable {
    public let data: Data
    public let documentID: UInt64
    public let documentRevision: UInt64

    public init(data: Data, documentID: UInt64, documentRevision: UInt64) {
        self.data = data
        self.documentID = documentID
        self.documentRevision = documentRevision
    }
}

/// The portion of core document state needed by NSDocument lifecycle chrome.
public struct EVDocumentPersistenceState: Equatable, Sendable {
    public var isDirty: Bool
    public var documentID: UInt64
    public var documentRevision: UInt64

    public init(
        isDirty: Bool = false,
        documentID: UInt64 = 0,
        documentRevision: UInt64 = 0
    ) {
        self.isDirty = isDirty
        self.documentID = documentID
        self.documentRevision = documentRevision
    }
}

/// A revision-tagged native document operation requested by one completed
/// core command turn. AppKit owns the dialogs and I/O; the core remains the
/// authority for the source bytes, dirty state, and command semantics.
public struct EVDocumentHostRequest: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case edit
        case new
        case write
        case saveAs
        case quit
        case quitAll
        case writeQuit
        case xit
        case writeAll
    }

    public let kind: Kind
    public let documentID: UInt64
    public let documentRevision: UInt64
    public let force: Bool
    public let path: String?
    public let hardLineRange: ClosedRange<UInt64>?

    public init(
        kind: Kind,
        documentID: UInt64,
        documentRevision: UInt64,
        force: Bool = false,
        path: String? = nil,
        hardLineRange: ClosedRange<UInt64>? = nil
    ) {
        self.kind = kind
        self.documentID = documentID
        self.documentRevision = documentRevision
        self.force = force
        self.path = path
        self.hardLineRange = hardLineRange
    }
}

/// Native document operations may present a panel or perform coordinated file
/// I/O, so completion is asynchronous even when a particular request closes a
/// window immediately.
@MainActor
public protocol EVDocumentHostEffectHandling: AnyObject {
    func perform(
        documentHostRequests: [EVDocumentHostRequest],
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    )
}

/// Optional editor-surface capability used by the AppKit shell to install the
/// document/window host without coupling the shell to the concrete editor.
@MainActor
public protocol EVDocumentHostAttachable: AnyObject {
    var documentHostEffectHandler: (any EVDocumentHostEffectHandling)? { get set }
}

public enum EVDocumentHostError: LocalizedError, Equatable {
    case staleRequest
    case documentModified
    case operationAlreadyInProgress
    case noDocumentURL
    case invalidPath(String)
    case saveCancelledOrFailed
    case preparedWriteUnavailable
    case unsupportedRequest

    public var errorDescription: String? {
        switch self {
        case .staleRequest:
            "The command referred to an older document revision."
        case .documentModified:
            "No write since the last change; add ! to override."
        case .operationAlreadyInProgress:
            "Another document operation is still in progress."
        case .noDocumentURL:
            "The document does not have a file location."
        case let .invalidPath(path):
            "The file path is invalid: \(path)"
        case .saveCancelledOrFailed:
            "The document was not saved."
        case .preparedWriteUnavailable:
            "Ranged and alternate-path writes require the prepared-write lifecycle, which is not available yet."
        case .unsupportedRequest:
            "That document request is not supported by the native frontend."
        }
    }
}

/// One view onto an editor document. Its NSView is a projection, never storage.
@MainActor
public protocol EVEditorSurface: AnyObject {
    var viewController: NSViewController { get }
    var statusBarState: EVStatusBarState { get }
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)? { get set }

    func perform(menuCommand: EVMenuCommand, sender: Any?)
    func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation
    func perform(statusOption: EVStatusBarOption)
}

public extension EVEditorSurface {
    func perform(statusOption: EVStatusBarOption) {}
}

/// One source-backed document/buffer. Multiple surfaces may share it.
@MainActor
public protocol EVDocumentBackend: AnyObject {
    var sourceDidChange: (() -> Void)? { get set }
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)? { get set }
    var persistenceState: EVDocumentPersistenceState { get }
    var sourceFormat: EVSourceFormat { get }

    func makeEditorSurface() -> any EVEditorSurface
    func read(source: Data, typeName: String) throws
    func serializedSource(typeName: String) throws -> Data
    func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot
    func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws
}

/// Installed by the frontend composition root before the first document opens.
@MainActor
public enum EVFrontendRegistry {
    private static var documentBackendFactory: (() -> any EVDocumentBackend)?

    public static func install(
        documentBackendFactory: @escaping () -> any EVDocumentBackend
    ) {
        self.documentBackendFactory = documentBackendFactory
    }

    static func makeDocumentBackend() -> any EVDocumentBackend {
        documentBackendFactory?() ?? EVUnavailableDocumentBackend()
    }
}

@MainActor
private final class EVUnavailableDocumentBackend: EVDocumentBackend {
    var sourceDidChange: (() -> Void)?
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
    var persistenceState = EVDocumentPersistenceState()
    private(set) var sourceFormat: EVSourceFormat = .plainText

    func makeEditorSurface() -> any EVEditorSurface {
        EVUnavailableEditorSurface()
    }

    func read(source: Data, typeName: String) throws {
        guard source.isEmpty else {
            throw EVAppShellError.frontendNotInstalled
        }
        sourceFormat = EVDocument.sourceFormat(forTypeName: typeName) ?? .plainText
    }

    func serializedSource(typeName: String) throws -> Data {
        Data()
    }

    func nativeSaveSnapshot(typeName _: String) throws -> EVDocumentSaveSnapshot {
        EVDocumentSaveSnapshot(data: Data(), documentID: 0, documentRevision: 0)
    }

    func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {}
}

@MainActor
private final class EVUnavailableEditorSurface: EVEditorSurface {
    let viewController: NSViewController
    var statusBarState = EVStatusBarState(message: "Editor backend unavailable")
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?

    init() {
        let label = NSTextField(wrappingLabelWithString: "The eVim editor surface was not installed.")
        label.alignment = .center
        label.textColor = .secondaryLabelColor

        let container = NSViewController()
        container.view = NSView()
        container.view.addSubview(label)
        label.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: container.view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: container.view.centerYAnchor),
            label.leadingAnchor.constraint(greaterThanOrEqualTo: container.view.leadingAnchor, constant: 24),
            label.trailingAnchor.constraint(lessThanOrEqualTo: container.view.trailingAnchor, constant: -24),
        ])
        viewController = container
    }

    func perform(menuCommand: EVMenuCommand, sender: Any?) {}

    func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation {
        .disabled
    }
}

public enum EVAppShellError: LocalizedError {
    case frontendNotInstalled

    public var errorDescription: String? {
        switch self {
        case .frontendNotInstalled:
            "The native editor backend is not installed."
        }
    }
}
