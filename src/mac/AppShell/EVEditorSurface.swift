import AppKit
import Foundation

public enum EVLineMode: UInt32, Equatable, Sendable {
  case visual = 0
  case physicalSource = 1
}

public enum EVStatusBarOption: Equatable, Sendable {
  case lineMode(EVLineMode)
}

/// The active `:`, `/`, or `?` command line. It is rendered inside the status
/// line, so its offsets travel as UTF-8 to match the core exactly.
public struct EVStatusCommandLine: Equatable, Sendable {
  public var prompt: String
  public var text: String
  /// Caret position within `text`.
  public var cursorUTF8Offset: Int
  /// UTF-16 ranges within `displayText`: input-method text still being
  /// composed, and the selection.
  public var markedDisplayRange: NSRange?
  public var selectedDisplayRange: NSRange?

  public init(
    prompt: String,
    text: String,
    cursorUTF8Offset: Int,
    markedDisplayRange: NSRange? = nil,
    selectedDisplayRange: NSRange? = nil
  ) {
    self.prompt = prompt
    self.text = text
    self.cursorUTF8Offset = cursorUTF8Offset
    self.markedDisplayRange = markedDisplayRange
    self.selectedDisplayRange = selectedDisplayRange
  }

  public var displayText: String { prompt + text }
}

/// Presentation-only values shown by the window's status bar.
public struct EVStatusBarState: Equatable, Sendable {
  public var mode: String
  public var message: String
  public var location: String
  public var lineMode: EVLineMode
  public var locationIsFragment: Bool
  /// While this is set, the command line replaces the status line's left
  /// group. The caret position widget stays.
  public var commandLine: EVStatusCommandLine?
  /// Read-only command output occupying the same area, in normal status colors.
  public var commandOutput: String?
  /// A pending core interaction must remain visible even with the status bar hidden.
  public var requiresInteraction: Bool
  /// Whether this pane holds the text focus. The command caret blinks only in
  /// the active pane and is outlined elsewhere.
  public var isActive: Bool

  public init(
    mode: String = "NORMAL",
    message: String = "",
    location: String = "Ln 1, Col 1",
    lineMode: EVLineMode = .visual,
    locationIsFragment: Bool = false,
    commandLine: EVStatusCommandLine? = nil,
    commandOutput: String? = nil,
    requiresInteraction: Bool = false,
    isActive: Bool = false
  ) {
    self.mode = mode
    self.message = message
    self.location = location
    self.lineMode = lineMode
    self.commandLine = commandLine
    self.commandOutput = commandOutput
    self.requiresInteraction = requiresInteraction
    self.isActive = isActive
    self.locationIsFragment = locationIsFragment
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
  public let isCompleteSource: Bool

  public init(data: Data, documentID: UInt64, documentRevision: UInt64, isCompleteSource: Bool = true) {
    self.data = data
    self.isCompleteSource = isCompleteSource
    self.documentID = documentID
    self.documentRevision = documentRevision
  }
}

/// The portion of core document state needed by NSDocument lifecycle chrome.
public struct EVDocumentPersistenceState: Equatable, Sendable {
  public var isDirty: Bool
  public var isReadOnly: Bool
  public var isRecovered: Bool
  public var documentID: UInt64
  public var documentRevision: UInt64
  /// Exact source length, when the backend can provide it without serialization.
  public var sourceByteCount: UInt64?

  public init(
    isDirty: Bool = false,
    isReadOnly: Bool = false,
    isRecovered: Bool = false,
    documentID: UInt64 = 0,
    documentRevision: UInt64 = 0,
    sourceByteCount: UInt64? = nil
  ) {
    self.isDirty = isDirty
    self.isReadOnly = isReadOnly
    self.isRecovered = isRecovered
    self.documentID = documentID
    self.documentRevision = documentRevision
    self.sourceByteCount = sourceByteCount
  }
}

/// A revision-tagged native document operation requested by one completed
/// core command turn. AppKit owns the dialogs and I/O; the core remains the
/// authority for the source bytes, dirty state, and command semantics.
public struct EVDocumentHostRequest: Equatable, Sendable {
  public enum Kind: Equatable, Sendable {
    case split
    case newPane
    case edit
    case editNewWindow
    case navigateArgument
    case printWorkingDirectory
    case checkTime
    case read
    case source
    case file
    case only
    case changeDirectory
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
  /// Initial text-area height of a new pane, measured in its visual rows.
  public let initialHeightRows: Int?
  public let argumentNavigation: EVArgumentNavigation?
  public let readAfterLine: UInt64?

  public init(
    kind: Kind,
    documentID: UInt64,
    documentRevision: UInt64,
    force: Bool = false,
    path: String? = nil,
    hardLineRange: ClosedRange<UInt64>? = nil,
    initialHeightRows: Int? = nil,
    argumentNavigation: EVArgumentNavigation? = nil,
    readAfterLine: UInt64? = nil
  ) {
    self.kind = kind
    self.documentID = documentID
    self.documentRevision = documentRevision
    self.force = force
    self.path = path
    self.hardLineRange = hardLineRange
    self.initialHeightRows = initialHeightRows
    self.argumentNavigation = argumentNavigation
    self.readAfterLine = readAfterLine
  }
}

/// Native document operations may present a panel or perform coordinated file
/// I/O, so completion is asynchronous even when a particular request closes a
/// window immediately.
@MainActor
public protocol EVDocumentHostEffectHandling: AnyObject {
  func documentURL(for surface: any EVEditorSurface) -> URL?
  func perform(
    documentHostRequests: [EVDocumentHostRequest],
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  )

  /// Pane geometry and focus belong to the host, not to the core or to any
  /// one surface. The requesting surface identifies the focused pane.
 
  func perform(windowRequests: [EVWindowRequest], from surface: any EVEditorSurface)

  /// A file drop targets the receiving surface, which may be an inactive pane.
  /// The host owns document identity, dirty-state review, and native windows.
  func openDroppedFiles(
    _ urls: [URL], in targetSurface: any EVEditorSurface,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  )
}

extension EVDocumentHostEffectHandling {
  public func documentURL(for surface: any EVEditorSurface) -> URL? { nil }
  public func perform(windowRequests: [EVWindowRequest], from surface: any EVEditorSurface) {}

  public func openDroppedFiles(
    _ urls: [URL], in targetSurface: any EVEditorSurface,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    completion(.failure(EVDocumentHostError.unsupportedRequest))
  }
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
  case destinationExists(String)
  case partialWriteRequiresForce
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
    case .invalidPath(let path):
      "The file path is invalid: \(path)"
    case .partialWriteRequiresForce:
      "Partial write to the current file requires !; the buffer remains modified."
    case .destinationExists(let path):
      "The file already exists: \(path). Add ! to overwrite it."
    case .saveCancelledOrFailed:
      "The document was not saved."
    case .preparedWriteUnavailable:
      "This formatted line range cannot be written as exact source bytes. Use a full-document write."
    case .unsupportedRequest:
      "That document request is not supported by the native frontend."
    }
  }
}

/// One `CTRL-W` window effect. Panes are ordered top to bottom, and an index
/// is one-based, matching the combined count around the prefix.
public enum EVWindowRequest: Equatable, Sendable {
  case focusDown(count: Int)
  case focusUp(count: Int)
  /// Without an index, the next pane, wrapping to the top.
  case focusNext(index: Int?)
  /// Without an index, the previous pane, wrapping to the bottom.
  case focusPrevious(index: Int?)
  case focusTop
  case focusBottom
  case focusLastAccessed
  case rotateDown(count: Int)
  case rotateUp(count: Int)
  /// Exchange the focused pane with the next one, with the previous one when
  /// it is last, or with the pane at `index`. Focus follows the pane.
  case exchange(index: Int?)
  case moveToTop
  case moveToBottom
  case closeOthers
  case grow(rows: Int)
  case shrink(rows: Int)
  /// Without a row count, as tall as the window allows.
  case setHeight(rows: Int?)
  case equalizeHeights
}

/// One view onto an editor document. Its NSView is a projection, never storage.
@MainActor
public protocol EVEditorSurface: AnyObject {
  var viewController: NSViewController { get }
  var statusBarState: EVStatusBarState { get }
  var statusBarStateDidChange: ((EVStatusBarState) -> Void)? { get set }

  /// Height of one laid-out visual row, the unit the `CTRL-W` height commands
  /// count in. `nil` before this surface has any layout.
  var visualRowHeight: CGFloat? { get }

  func perform(menuCommand: EVMenuCommand, sender: Any?)
  func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation
  func perform(statusOption: EVStatusBarOption)
  func showDocumentMessage(_ message: String)
  /// A complete styled HTML copy of this view, without changing document state.
  func htmlExportData() async throws -> Data
  /// Startup line in the first argument; UInt64.max selects its last line.
  func goToLine(_ line: UInt64)
  func insertFileContents(_ bytes: Data, after: UInt64, expected: EVDocumentPersistenceState) throws
  func executeSourcedLine(_ text: String, depth: UInt32) throws -> [EVDocumentHostRequest]
  func dismissCommandOutput()
  /// Resume editor input after leaving the selectable status message.
  func handleStatusMessageKey(_ event: NSEvent)

  /// Place the command-line caret, which the status line hit-tested against
  /// its own rendering. Offsets are UTF-8 within the command-line text.
  func selectCommandLine(atUTF8Offset offset: Int, extending: Bool)
}

extension EVEditorSurface {
  public var visualRowHeight: CGFloat? { nil }
  public func perform(statusOption: EVStatusBarOption) {}
  public func selectCommandLine(atUTF8Offset offset: Int, extending: Bool) {}
  public func showDocumentMessage(_ message: String) {}
  public func htmlExportData() async throws -> Data { throw EVDocumentHostError.unsupportedRequest }
  public func goToLine(_ line: UInt64) {}
  public func insertFileContents(_ bytes: Data, after: UInt64, expected: EVDocumentPersistenceState) throws { throw EVDocumentHostError.unsupportedRequest }
  public func executeSourcedLine(_ text: String, depth: UInt32) throws -> [EVDocumentHostRequest] { throw EVDocumentHostError.unsupportedRequest }
  public func dismissCommandOutput() {}
  public func handleStatusMessageKey(_ event: NSEvent) {}
}

/// One source-backed document/buffer. Multiple surfaces may share it.
@MainActor
public protocol EVDocumentBackend: AnyObject {
  var sourceDidChange: (() -> Void)? { get set }
  var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)? { get set }
  var persistenceState: EVDocumentPersistenceState { get }
  var sourceFormat: EVSourceFormat { get }
  var prefersMarkdownFormattedView: Bool { get }

  func makeEditorSurface() -> any EVEditorSurface
  func read(source: Data, typeName: String) throws
  func read(source: Data, typeName: String, filename: String?, allowAutomaticCode: Bool) throws
  func updateFilename(_ filename: String)
  func serializedSource(typeName: String) throws -> Data
  func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot
  func nativeSaveSnapshot(typeName: String, hardLineRange: ClosedRange<UInt64>) throws -> EVDocumentSaveSnapshot
  func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws
  func recoverySnapshot() throws -> EVRecoverySnapshot
  func restoreRecovery(_ snapshot: EVRecoverySnapshot) throws
  func setReadOnly(_ readOnly: Bool) throws
}

extension EVDocumentBackend {
  public var prefersMarkdownFormattedView: Bool { false }
  public func updateFilename(_ filename: String) {}
  public func read(source: Data, typeName: String, filename: String?, allowAutomaticCode: Bool) throws {
    try read(source: source, typeName: typeName)
  }
  public func nativeSaveSnapshot(typeName: String, hardLineRange: ClosedRange<UInt64>) throws -> EVDocumentSaveSnapshot { throw EVDocumentHostError.preparedWriteUnavailable }
  public func recoverySnapshot() throws -> EVRecoverySnapshot {
    throw EVRecoveryError.backendUnavailable
  }
  public func restoreRecovery(_ snapshot: EVRecoverySnapshot) throws {
    throw EVRecoveryError.backendUnavailable
  }
  public func setReadOnly(_ readOnly: Bool) throws {
    if readOnly { throw EVRecoveryError.backendUnavailable }
  }
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
  var persistenceState = EVDocumentPersistenceState(sourceByteCount: 0)
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
    let label = NSTextField(wrappingLabelWithString: "The Viem editor surface was not installed.")
    label.alignment = .center
    label.textColor = .secondaryLabelColor

    let container = NSViewController()
    container.view = NSView()
    container.view.addSubview(label)
    label.translatesAutoresizingMaskIntoConstraints = false
    NSLayoutConstraint.activate([
      label.centerXAnchor.constraint(equalTo: container.view.centerXAnchor),
      label.centerYAnchor.constraint(equalTo: container.view.centerYAnchor),
      label.leadingAnchor.constraint(
        greaterThanOrEqualTo: container.view.leadingAnchor, constant: 24),
      label.trailingAnchor.constraint(
        lessThanOrEqualTo: container.view.trailingAnchor, constant: -24),
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
