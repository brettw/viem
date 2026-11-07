import AppKit
import CViemCore

// Diff mode (docs/diff_mode.md): a window shows a read-only base version of
// its document beside the live buffer, which stays editable in Code. The shell
// owns the toolbar, base loading, the file list and the side-by-side layout;
// the editor supplies the base buffer and the comparison session.

/// A document backend that can take part in diff mode.
@MainActor
public protocol EVDiffBackend: AnyObject {
  /// The source as last loaded or saved, from memory; nil when unknown.
  func diffSavedSource() -> Data?
  /// A read-only, non-modifiable Code buffer showing `source`, with this
  /// document's language.
  func makeDiffBase(source: Data) throws -> any EVDocumentBackend
  /// Compare `base` (shown by `baseSurface`) with this document (shown by
  /// `surface`). The session shows the document in Code until it ends.
  /// `editable` is false for a file deleted since the base.
  func beginDiff(base: any EVDocumentBackend, baseSurface: any EVEditorSurface,
                 surface: any EVEditorSurface, aligned: Bool, editable: Bool) throws -> any EVDiffSessionControlling
}

/// One live comparison, owned by the window's diff mode.
@MainActor
public protocol EVDiffSessionControlling: AnyObject {
  func setAligned(_ aligned: Bool)
  /// Move both sides to the next or previous change, wrapping within the file.
  func navigateChange(next: Bool)
  /// Remove the presentation and restore the document's previous mode.
  func end()
}

/// Line-number control for both sides of a diff.
@MainActor
public protocol EVDiffLineNumberControlling: AnyObject {
  var showsLineNumbers: Bool { get }
  func setShowsLineNumbers(_ shows: Bool)
}

/// The version a window's diff compares with.
public enum EVDiffBase: Equatable, Sendable {
  case lastSaved
  case unstaged
  case head
  case branch(String)
  case file(URL)

  var title: String {
    switch self {
    case .lastSaved: "Last Saved"
    case .unstaged: "Unstaged"
    case .head: "HEAD"
    case let .branch(name): "Branch: \(name)"
    case let .file(url): "File: \(url.lastPathComponent)"
    }
  }

  var isGit: Bool {
    switch self {
    case .unstaged, .head, .branch: true
    case .lastSaved, .file: false
    }
  }

  nonisolated var kind: UInt32 {
    switch self {
    case .lastSaved: UInt32(VIEM_DIFF_BASE_LAST_SAVED)
    case .unstaged: UInt32(VIEM_DIFF_BASE_UNSTAGED)
    case .head: UInt32(VIEM_DIFF_BASE_HEAD)
    case .branch: UInt32(VIEM_DIFF_BASE_BRANCH)
    case .file: UInt32(VIEM_DIFF_BASE_FILE)
    }
  }
}

/// One entry of the diff's file list.
public struct EVDiffFile: Equatable, Sendable {
  public var url: URL
  /// Where the base version lives; differs from `url` for a rename.
  public var baseURL: URL
  public var change: UInt32

  var isDeleted: Bool { change == UInt32(VIEM_DIFF_FILE_DELETED) }

  var title: String {
    let name = url.lastPathComponent
    switch change {
    case UInt32(VIEM_DIFF_FILE_RENAMED): return "\(baseURL.lastPathComponent) → \(name)"
    case UInt32(VIEM_DIFF_FILE_DELETED): return "\(name) (deleted)"
    case UInt32(VIEM_DIFF_FILE_ADDED): return "\(name) (added)"
    case UInt32(VIEM_DIFF_FILE_UNSAVED): return "\(name) (unsaved)"
    default: return name
    }
  }
}

struct EVDiffLoadFailure: Error, Sendable, LocalizedError {
  var result: UInt32
  var message: String
  var errorDescription: String? { message.isEmpty ? "The diff base is unavailable." : message }
}

/// Blocking base-source calls through the core, run off the main thread.
enum EVDiffBaseLoader {
  struct Loaded: Sendable {
    var bytes: Data
    var exists: Bool
  }

  static func load(_ base: EVDiffBase, path: String) async -> Result<Loaded, EVDiffLoadFailure> {
    await Task.detached(priority: .userInitiated) { loadBlocking(base, path: path) }.value
  }

  /// `scope` limits a git list to one folder, as `git diff -- <folder>`.
  static func listFiles(_ base: EVDiffBase, path: String, unsaved: [String],
                        scope: String? = nil) async -> Result<[EVDiffFile], EVDiffLoadFailure> {
    await Task.detached(priority: .userInitiated) {
      listFilesBlocking(base, path: path, unsaved: unsaved, scope: scope)
    }.value
  }

  static func listBranches(path: String) async -> Result<[String], EVDiffLoadFailure> {
    await Task.detached(priority: .userInitiated) {
      withResult(.head, path: path, unsaved: [], call: viem_diff_list_branches) { handle, info in
        (0..<info.item_count).compactMap { item(handle, $0)?.path }
      }
    }.value
  }

  nonisolated static func loadBlocking(_ base: EVDiffBase, path: String) -> Result<Loaded, EVDiffLoadFailure> {
    withResult(base, path: path, unsaved: [], call: viem_diff_load_base) { handle, info in
      var bytes = Data(count: Int(info.byte_count))
      var required: UInt64 = 0
      let status = bytes.withUnsafeMutableBytes { buffer in
        viem_diff_result_copy_bytes(handle, buffer.bindMemory(to: UInt8.self).baseAddress,
                                    UInt64(buffer.count), &required)
      }
      if status != UInt32(VIEM_STATUS_OK) { bytes = Data() }
      return Loaded(bytes: bytes, exists: info.flags & UInt32(VIEM_DIFF_RESULT_BASE_EXISTS) != 0)
    }
  }

  nonisolated static func listFilesBlocking(_ base: EVDiffBase, path: String, unsaved: [String],
                                            scope: String? = nil) -> Result<[EVDiffFile], EVDiffLoadFailure> {
    withResult(base, path: path, unsaved: unsaved, scope: scope, call: viem_diff_list_files) { handle, info in
      (0..<info.item_count).compactMap { index in
        guard let entry = item(handle, index) else { return nil }
        let url = URL(fileURLWithPath: entry.path)
        let baseURL = entry.basePath.isEmpty ? url : URL(fileURLWithPath: entry.basePath)
        return EVDiffFile(url: url, baseURL: baseURL, change: entry.change)
      }
    }
  }

  private nonisolated static func item(_ handle: ViemDiffResultHandle, _ index: UInt64) -> (path: String, basePath: String, change: UInt32)? {
    var entry = ViemDiffListItemV1()
    entry.struct_size = UInt32(MemoryLayout<ViemDiffListItemV1>.size)
    var required: UInt64 = 0
    var text = [UInt8](repeating: 0, count: 4096)
    var status = viem_diff_result_copy_item(handle, index, &entry, &text, UInt64(text.count), &required)
    if status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) {
      text = [UInt8](repeating: 0, count: Int(required))
      status = viem_diff_result_copy_item(handle, index, &entry, &text, UInt64(text.count), &required)
    }
    guard status == UInt32(VIEM_STATUS_OK) else { return nil }
    let split = Int(entry.path_bytes)
    let path = String(decoding: text[0..<split], as: UTF8.self)
    let basePath = String(decoding: text[split..<split + Int(entry.base_path_bytes)], as: UTF8.self)
    return (path, basePath, entry.change)
  }

  /// Build one request, call the core, read the owned result and release it.
  private nonisolated static func withResult<T>(
    _ base: EVDiffBase, path: String, unsaved: [String], scope: String? = nil,
    call: (UnsafePointer<ViemDiffBaseRequestV1>?, UnsafeMutablePointer<ViemDiffResultHandle>?) -> ViemStatus,
    read: (ViemDiffResultHandle, ViemDiffResultInfoV1) -> T
  ) -> Result<T, EVDiffLoadFailure> {
    let filePath: String
    let branch: String
    switch base {
    case let .file(url): filePath = url.path; branch = ""
    case let .branch(name): filePath = path; branch = name
    default: filePath = path; branch = ""
    }
    let strings = [filePath, branch, scope ?? ""] + unsaved
    let buffers = strings.map { string -> UnsafeMutableBufferPointer<UInt8> in
      let bytes = Array(string.utf8)
      let buffer = UnsafeMutableBufferPointer<UInt8>.allocate(capacity: max(1, bytes.count))
      _ = buffer.initialize(from: bytes)
      return buffer
    }
    defer { buffers.forEach { $0.deallocate() } }
    func slice(_ index: Int) -> ViemUtf8Slice {
      ViemUtf8Slice(data: UnsafePointer(buffers[index].baseAddress), length: UInt64(strings[index].utf8.count))
    }
    var unsavedSlices = (3..<strings.count).map(slice)
    var request = ViemDiffBaseRequestV1()
    request.struct_size = UInt32(MemoryLayout<ViemDiffBaseRequestV1>.size)
    request.kind = base.kind
    request.file_path = slice(0)
    request.branch = slice(1)
    request.scope_path = slice(2)
    var handle: ViemDiffResultHandle = 0
    let status = unsavedSlices.withUnsafeMutableBufferPointer { slices -> ViemStatus in
      request.unsaved_paths = UnsafePointer(slices.baseAddress)
      request.unsaved_path_count = UInt64(slices.count)
      return call(&request, &handle)
    }
    guard status == UInt32(VIEM_STATUS_OK), handle != 0 else {
      return .failure(EVDiffLoadFailure(result: UInt32(VIEM_DIFF_RESULT_UNAVAILABLE),
                                        message: "The diff base request was rejected (\(status))."))
    }
    defer { _ = viem_diff_result_release(handle) }
    var info = ViemDiffResultInfoV1()
    info.struct_size = UInt32(MemoryLayout<ViemDiffResultInfoV1>.size)
    guard viem_diff_result_info(handle, &info) == UInt32(VIEM_STATUS_OK) else {
      return .failure(EVDiffLoadFailure(result: UInt32(VIEM_DIFF_RESULT_UNAVAILABLE), message: ""))
    }
    guard info.result == UInt32(VIEM_DIFF_RESULT_OK) else {
      var message = [UInt8](repeating: 0, count: Int(info.message_bytes))
      var required: UInt64 = 0
      _ = viem_diff_result_copy_message(handle, &message, UInt64(message.count), &required)
      return .failure(EVDiffLoadFailure(result: info.result, message: String(decoding: message, as: UTF8.self)))
    }
    return .success(read(handle, info))
  }
}

/// The left (base) and right (document) halves of a diff window.
@MainActor
final class EVDiffSplitView: NSView {
  let base: NSView
  let document: NSView
  private let divider = NSBox()

  init(base: NSView, document: NSView) {
    self.base = base
    self.document = document
    super.init(frame: .zero)
    divider.boxType = .separator
    for view in [base, divider, document] {
      view.translatesAutoresizingMaskIntoConstraints = true
      addSubview(view)
    }
    setAccessibilityElement(false)
  }

  @available(*, unavailable) required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  override func layout() {
    super.layout()
    let half = (bounds.width - 1) / 2
    base.frame = NSRect(x: 0, y: 0, width: half.rounded(.down), height: bounds.height)
    divider.frame = NSRect(x: base.frame.maxX, y: 0, width: 1, height: bounds.height)
    document.frame = NSRect(x: divider.frame.maxX, y: 0, width: bounds.width - divider.frame.maxX, height: bounds.height)
  }
}

/// The window's diff toolbar, shown below the title bar in diff mode.
@MainActor
final class EVDiffToolbar: NSObject {
  let accessory = NSTitlebarAccessoryViewController()
  let basePopup = NSPopUpButton(frame: .zero, pullsDown: false)
  let chooseFileButton = NSButton()
  let refreshButton = NSButton()
  let previousButton = NSButton()
  let filePopup = NSPopUpButton(frame: .zero, pullsDown: false)
  let nextButton = NSButton()
  let alignButton = NSButton()
  let previousChangeButton = NSButton()
  let nextChangeButton = NSButton()
  var selectBase: ((Int) -> Void)?
  var chooseFile: (() -> Void)?
  var refresh: (() -> Void)?
  var previous: (() -> Void)?
  var next: (() -> Void)?
  var selectFile: ((Int) -> Void)?
  var toggleAlign: (() -> Void)?
  var previousChange: (() -> Void)?
  var nextChange: (() -> Void)?

  /// An arrow pointing into a change block: below it for Next change, above
  /// it for Previous change. A template image, so it follows the appearance.
  static func changeIcon(next: Bool) -> NSImage {
    let image = NSImage(size: NSSize(width: 16, height: 16), flipped: !next) { _ in
      NSColor.black.setFill()
      // Drawn for Next (block at the bottom); Previous flips the canvas.
      NSBezierPath(roundedRect: NSRect(x: 3, y: 1.5, width: 10, height: 4), xRadius: 1.2, yRadius: 1.2).fill()
      NSBezierPath(rect: NSRect(x: 7.2, y: 9.5, width: 1.6, height: 5)).fill()
      let head = NSBezierPath()
      head.move(to: NSPoint(x: 4.5, y: 10))
      head.line(to: NSPoint(x: 11.5, y: 10))
      head.line(to: NSPoint(x: 8, y: 6.6))
      head.close()
      head.fill()
      return true
    }
    image.isTemplate = true
    image.accessibilityDescription = next ? "Next Change" : "Previous Change"
    return image
  }

  override init() {
    super.init()
    accessory.layoutAttribute = .bottom
    let stack = NSStackView()
    stack.orientation = .horizontal
    stack.spacing = 6
    stack.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 6, right: 10)
    func configure(_ button: NSButton, symbol: String, label: String, action: Selector) {
      button.bezelStyle = .accessoryBarAction
      button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
      button.imagePosition = .imageOnly
      button.title = ""
      button.toolTip = label
      button.setAccessibilityLabel(label)
      button.target = self
      button.action = action
      button.refusesFirstResponder = true
    }
    basePopup.target = self
    basePopup.action = #selector(baseChanged(_:))
    basePopup.toolTip = "Diff base"
    basePopup.setAccessibilityLabel("Diff base")
    configure(chooseFileButton, symbol: "doc.badge.ellipsis", label: "Choose Base File…", action: #selector(chooseFileClicked(_:)))
    configure(refreshButton, symbol: "arrow.clockwise", label: "Refresh Base", action: #selector(refreshClicked(_:)))
    configure(previousButton, symbol: "chevron.left", label: "Previous File", action: #selector(previousClicked(_:)))
    configure(nextButton, symbol: "chevron.right", label: "Next File", action: #selector(nextClicked(_:)))
    filePopup.target = self
    filePopup.action = #selector(fileChanged(_:))
    filePopup.toolTip = "Files in this diff"
    filePopup.setAccessibilityLabel("Files in this diff")
    alignButton.setButtonType(.switch)
    alignButton.title = "Align Sides"
    alignButton.target = self
    alignButton.action = #selector(alignClicked(_:))
    alignButton.refusesFirstResponder = true
    for (button, next) in [(previousChangeButton, false), (nextChangeButton, true)] {
      let label = next ? "Next Change" : "Previous Change"
      configure(button, symbol: "circle", label: label,
                action: next ? #selector(nextChangeClicked(_:)) : #selector(previousChangeClicked(_:)))
      button.image = Self.changeIcon(next: next)
    }
    filePopup.setContentHuggingPriority(.defaultLow, for: .horizontal)
    filePopup.widthAnchor.constraint(greaterThanOrEqualToConstant: 220).isActive = true
    for view in [basePopup, chooseFileButton, refreshButton, previousButton, filePopup, nextButton, alignButton,
                 previousChangeButton, nextChangeButton] as [NSView] {
      stack.addArrangedSubview(view)
    }
    stack.frame = NSRect(x: 0, y: 0, width: 800, height: 34)
    stack.autoresizingMask = [.width]
    let container = NSView(frame: stack.frame)
    container.addSubview(stack)
    accessory.view = container
  }

  @objc private func baseChanged(_ sender: NSPopUpButton) { selectBase?(sender.indexOfSelectedItem) }
  @objc private func chooseFileClicked(_ sender: Any?) { chooseFile?() }
  @objc private func refreshClicked(_ sender: Any?) { refresh?() }
  @objc private func previousClicked(_ sender: Any?) { previous?() }
  @objc private func nextClicked(_ sender: Any?) { next?() }
  @objc private func fileChanged(_ sender: NSPopUpButton) { selectFile?(sender.indexOfSelectedItem) }
  @objc private func alignClicked(_ sender: Any?) { toggleAlign?() }
  @objc private func previousChangeClicked(_ sender: Any?) { previousChange?() }
  @objc private func nextChangeClicked(_ sender: Any?) { nextChange?() }
}

/// Folder diffs ("Diff <directory>" and "Diff Folder…").
enum EVDiffFolder {
  /// The longest directory path the menu names before clipping it.
  static let menuPathLimit = 40

  /// "Diff <directory>", the path shortened from the left with a leading
  /// ellipsis to at most `menuPathLimit` characters.
  static func menuTitle(forDirectory path: String) -> String {
    let shown = path.count > menuPathLimit ? "…" + path.suffix(menuPathLimit - 1) : Substring(path)
    return "Diff \(shown)"
  }
}

/// What a window needs from its controller to run diff mode.
@MainActor
protocol EVDiffModeHost: AnyObject {
  var diffWindow: NSWindow? { get }
  var diffActiveDocument: EVDocument? { get }
  var diffActiveSurface: (any EVEditorSurface)? { get }
  var diffDocumentContentView: NSView { get }
  var diffOpenPanelPresenter: ((NSOpenPanel, NSWindow, @escaping @MainActor (URL?) -> Void) -> Void)? { get }
  /// Show `view` as the window's content; nil restores the pane container.
  func installDiffContent(_ view: NSView?)
  /// Replace the active pane's document with `url`, discarding unsaved
  /// changes the caller has already reviewed.
  func openDiffFile(_ url: URL, completion: @escaping @MainActor (Error?) -> Void)
  /// Unsaved open documents, for the file list.
  func unsavedDocumentURLs() -> [URL]
}

/// One window's diff mode: its base, file list, toolbar and comparison.
@MainActor
final class EVDiffModeController: NSObject {
  private weak var host: (any EVDiffModeHost)?
  private let configuration: EVConfigurationStore
  private let toolbar = EVDiffToolbar()
  private(set) var isEnabled = false
  private(set) var base: EVDiffBase = .lastSaved
  private(set) var files: [EVDiffFile] = []
  private(set) var branches: [String] = []
  private(set) var isAligned: Bool
  /// The folder a folder diff lists; nil lists the whole repository.
  private(set) var scope: URL?
  private var baseChoices: [EVDiffBase] = []
  private var baseBackend: (any EVDocumentBackend)?
  private var baseSurface: (any EVEditorSurface)?
  private var session: (any EVDiffSessionControlling)?
  private weak var presentedDocument: EVDocument?
  private weak var presentedSurface: AnyObject?
  /// Retires asynchronous loads that a newer request superseded.
  private var generation = 0
  /// Layout during an install can report an active-document change; the
  /// document being installed is already the presented one.
  private var isInstalling = false
  private(set) var lastMessage: String?
  /// Tests await this after a change to know the async work is installed.
  var didPresent: (() -> Void)?

  init(host: any EVDiffModeHost, configuration: EVConfigurationStore) {
    self.host = host
    self.configuration = configuration
    isAligned = configuration.diffAlignSides
    super.init()
    toolbar.selectBase = { [weak self] index in self?.selectBase(at: index) }
    toolbar.chooseFile = { [weak self] in self?.chooseBaseFile() }
    toolbar.refresh = { [weak self] in self?.refresh() }
    toolbar.previous = { [weak self] in self?.moveFile(by: -1) }
    toolbar.next = { [weak self] in self?.moveFile(by: 1) }
    toolbar.selectFile = { [weak self] index in self?.selectFile(at: index) }
    toolbar.toggleAlign = { [weak self] in self?.toggleAlign() }
    toolbar.previousChange = { [weak self] in self?.navigateChange(next: false) }
    toolbar.nextChange = { [weak self] in self?.navigateChange(next: true) }
  }

  // MARK: Entering and leaving

  func toggle() {
    if isEnabled { disable() } else { enable() }
  }

  /// Turn diff mode on and show the toolbar; false when it cannot start.
  private func begin() -> Bool {
    guard !isEnabled, let host, let window = host.diffWindow, host.diffActiveDocument != nil else { return false }
    isEnabled = true
    isAligned = configuration.diffAlignSides
    if !window.titlebarAccessoryViewControllers.contains(where: { $0 === toolbar.accessory }) {
      window.addTitlebarAccessoryViewController(toolbar.accessory)
    }
    return true
  }

  /// Compare the files `git diff -- <folder>` lists, starting with the
  /// active document (normally the first of `files`).
  func enable(folder: URL, files: [EVDiffFile]) {
    guard isEnabled || begin() else { return }
    scope = folder
    base = .unstaged
    self.files = files
    generation += 1
    present()
  }

  func enable() {
    guard let host, begin() else { return }
    scope = nil
    let url = host.diffActiveDocument?.fileURL
    generation += 1
    let current = generation
    // Default to `git diff`, falling back to Last saved outside a repository.
    guard let url else {
      base = .lastSaved
      present()
      return
    }
    Task { @MainActor in
      let listed = await EVDiffBaseLoader.listFiles(.unstaged, path: url.path, unsaved: self.unsavedPaths())
      guard current == self.generation, self.isEnabled else { return }
      switch listed {
      case let .success(files):
        self.base = .unstaged
        self.files = files
      case .failure:
        self.base = .lastSaved
        self.files = [EVDiffFile(url: url, baseURL: url, change: UInt32(VIEM_DIFF_FILE_MODIFIED))]
      }
      self.present()
    }
  }

  func disable() {
    guard isEnabled else { return }
    isEnabled = false
    generation += 1
    tearDownComparison()
    if let surface = host?.diffActiveSurface as? any EVDiffLineNumberControlling,
       let code = configuration.codeLineNumbers {
      surface.setShowsLineNumbers(code)
    }
    host?.installDiffContent(nil)
    if let window = host?.diffWindow,
       let index = window.titlebarAccessoryViewControllers.firstIndex(where: { $0 === toolbar.accessory }) {
      window.removeTitlebarAccessoryViewController(at: index)
    }
    files = []
    scope = nil
    presentedDocument = nil
  }

  private func tearDownComparison() {
    session?.end()
    session = nil
    baseSurface = nil
    baseBackend = nil
    presentedSurface = nil
  }

  /// The active document changed by another route (`:e`, a link, a pane
  /// switch): compare it with the same base.
  func activeDocumentDidChange() {
    guard isEnabled, !isInstalling, let host else { return }
    if host.diffActiveDocument !== presentedDocument || host.diffActiveSurface as AnyObject? !== presentedSurface {
      present()
    }
  }

  // MARK: Presenting

  private func basePath(for document: EVDocument) -> String? {
    guard let url = document.fileURL else { return nil }
    return files.first(where: { $0.url.standardizedFileURL == url.standardizedFileURL })?.baseURL.path ?? url.path
  }

  /// Load the base for the active document and show the comparison.
  func present(readingDisk: Bool = false) {
    guard isEnabled, let host, let document = host.diffActiveDocument else { return }
    generation += 1
    let current = generation
    refreshToolbar()
    let base = self.base
    if base == .lastSaved, !readingDisk,
       let saved = (document.editorBackend as? any EVDiffBackend)?.diffSavedSource() {
      install(base: saved, for: document)
      return
    }
    let path: String
    if case let .file(url) = base {
      path = url.path
    } else if let documentPath = basePath(for: document) {
      path = documentPath
    } else {
      // An untitled document's Last saved base is empty.
      install(base: Data(), for: document)
      return
    }
    let deleted = files.contains { $0.isDeleted && $0.url.path == document.fileURL?.path }
    Task { @MainActor in
      let loaded = await EVDiffBaseLoader.load(base, path: path)
      guard current == self.generation, self.isEnabled, host.diffActiveDocument === document else { return }
      switch loaded {
      case let .success(result): self.install(base: result.bytes, for: document, editable: !deleted)
      case let .failure(error):
        self.lastMessage = error.localizedDescription
        host.diffActiveSurface?.showDocumentMessage(error.localizedDescription)
        self.install(base: Data(), for: document, editable: !deleted)
      }
    }
  }

  private func install(base bytes: Data, for document: EVDocument, editable: Bool = true) {
    guard !isInstalling, let host, let surface = host.diffActiveSurface,
          let backend = document.editorBackend as? any EVDiffBackend else { return }
    isInstalling = true
    defer { isInstalling = false }
    tearDownComparison()
    presentedDocument = document
    presentedSurface = surface
    do {
      let baseBackend = try backend.makeDiffBase(source: bytes)
      let baseSurface = baseBackend.makeEditorSurface()
      let split = EVDiffSplitView(base: baseSurface.viewController.view, document: host.diffDocumentContentView)
      host.installDiffContent(split)
      split.layoutSubtreeIfNeeded()
      session = try backend.beginDiff(base: baseBackend, baseSurface: baseSurface, surface: surface,
                                      aligned: isAligned, editable: editable)
      let numbers = configuration.diffLineNumbers
      (baseSurface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(numbers)
      (surface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(numbers)
      self.baseBackend = baseBackend
      self.baseSurface = baseSurface
      presentedDocument = document
      presentedSurface = surface
      lastMessage = nil
    } catch {
      lastMessage = error.localizedDescription
      surface.showDocumentMessage(error.localizedDescription)
    }
    refreshToolbar()
    didPresent?()
  }

  // MARK: Toolbar and menu actions

  func refresh() {
    guard isEnabled, let host, let document = host.diffActiveDocument else { return }
    reloadFiles(for: document) { [weak self] in self?.present(readingDisk: true) }
  }

  private func reloadFiles(for document: EVDocument, then: @escaping @MainActor () -> Void) {
    let base = self.base
    guard let url = document.fileURL else { files = []; then(); return }
    switch base {
    case .file:
      files = []
      then()
    case .lastSaved:
      files = [EVDiffFile(url: url, baseURL: url, change: UInt32(VIEM_DIFF_FILE_MODIFIED))]
      then()
    default:
      let current = generation
      Task { @MainActor in
        let listed = await EVDiffBaseLoader.listFiles(base, path: url.path, unsaved: self.unsavedPaths(),
                                                      scope: self.scope?.path)
        guard current == self.generation, self.isEnabled else { return }
        switch listed {
        case let .success(files): self.files = files
        case let .failure(error):
          self.files = []
          self.lastMessage = error.localizedDescription
          self.host?.diffActiveSurface?.showDocumentMessage(error.localizedDescription)
        }
        then()
      }
    }
  }

  func setBase(_ next: EVDiffBase) {
    guard isEnabled, let document = host?.diffActiveDocument else { return }
    base = next
    generation += 1
    reloadFiles(for: document) { [weak self] in self?.present() }
  }

  private func selectBase(at index: Int) {
    guard baseChoices.indices.contains(index) else { return }
    let choice = baseChoices[index]
    if case .branch("") = choice {
      chooseBranch()
    } else {
      setBase(choice)
    }
  }

  private func chooseBranch() {
    guard let path = host?.diffActiveDocument?.fileURL?.path else { return }
    Task { @MainActor in
      let listed = await EVDiffBaseLoader.listBranches(path: path)
      switch listed {
      case let .success(branches):
        self.branches = branches
        self.refreshToolbar()
        self.toolbar.basePopup.performClick(nil)
      case let .failure(error):
        self.host?.diffActiveSurface?.showDocumentMessage(error.localizedDescription)
        self.refreshToolbar()
      }
    }
  }

  func chooseBaseFile() {
    guard isEnabled, let host, let window = host.diffWindow else { return }
    let panel = NSOpenPanel()
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    // Every file is a valid base, including extensionless files and dotfiles.
    panel.allowedContentTypes = []
    panel.showsHiddenFiles = true
    panel.message = "Choose a file to compare with"
    let selected: @MainActor (URL?) -> Void = { [weak self] url in
      guard let self, let url else { return }
      self.setBase(.file(url))
    }
    if let presenter = host.diffOpenPanelPresenter {
      presenter(panel, window, selected)
    } else {
      panel.beginSheetModal(for: window) { response in
        MainActor.assumeIsolated { selected(response == .OK ? panel.url : nil) }
      }
    }
  }

  func toggleAlign() {
    isAligned.toggle()
    try? configuration.setDiffAlignSides(isAligned)
    session?.setAligned(isAligned)
    refreshToolbar()
  }

  /// Move both sides to the next or previous change in this file.
  func navigateChange(next: Bool) {
    guard isEnabled else { return }
    session?.navigateChange(next: next)
  }

  /// A line-number toggle on either side applies to both and is saved.
  func lineNumbersDidToggle(on surface: any EVEditorSurface) {
    guard isEnabled, let shows = (surface as? any EVDiffLineNumberControlling)?.showsLineNumbers else { return }
    try? configuration.setDiffLineNumbers(shows)
    (baseSurface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(shows)
    (host?.diffActiveSurface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(shows)
  }

  var currentFileIndex: Int? {
    guard let url = host?.diffActiveDocument?.fileURL?.standardizedFileURL else { return nil }
    return files.firstIndex { $0.url.standardizedFileURL == url }
  }

  func moveFile(by step: Int) {
    guard isEnabled, !files.isEmpty else { return }
    let index: Int
    if let current = currentFileIndex {
      index = ((current + step) % files.count + files.count) % files.count
    } else {
      index = step > 0 ? 0 : files.count - 1
    }
    selectFile(at: index)
  }

  private func selectFile(at index: Int) {
    guard isEnabled, files.indices.contains(index), let host, let document = host.diffActiveDocument else { return }
    let target = files[index].url
    guard target.standardizedFileURL != document.fileURL?.standardizedFileURL else { return }
    reviewUnsavedChanges(of: document) { [weak self] proceed in
      guard let self, proceed, self.isEnabled else { self?.refreshToolbar(); return }
      host.openDiffFile(target) { error in
        if let error { host.diffActiveSurface?.showDocumentMessage(error.localizedDescription) }
        self.present()
      }
    }
  }

  /// The save prompt a window close shows: Save, Don't Save or Cancel.
  private var pendingReview: ((Bool) -> Void)?
  private func reviewUnsavedChanges(of document: EVDocument, completion: @escaping (Bool) -> Void) {
    guard document.editorBackend.persistenceState.isDirty || document.isDocumentEdited else {
      completion(true)
      return
    }
    pendingReview = completion
    document.canClose(withDelegate: self, shouldClose: #selector(reviewed(_:shouldClose:contextInfo:)), contextInfo: nil)
  }

  @objc private func reviewed(_ document: NSDocument, shouldClose: Bool, contextInfo: UnsafeMutableRawPointer?) {
    let completion = pendingReview
    pendingReview = nil
    completion?(shouldClose)
  }

  private func unsavedPaths() -> [String] {
    (host?.unsavedDocumentURLs() ?? []).map(\.path)
  }

  // MARK: Toolbar state

  func refreshToolbar() {
    let fileURL = host?.diffActiveDocument?.fileURL
    var choices: [EVDiffBase] = [.lastSaved]
    if fileURL != nil { choices += [.unstaged, .head] }
    choices += branches.map { .branch($0) }
    if fileURL != nil { choices.append(.branch("")) }
    if case .file = base { choices.append(base) }
    if case .branch = base, !choices.contains(base) { choices.insert(base, at: min(3, choices.count)) }
    baseChoices = choices
    let popup = toolbar.basePopup
    popup.removeAllItems()
    for choice in choices {
      if case .branch("") = choice { popup.addItem(withTitle: "Branch…") } else { popup.addItem(withTitle: choice.title) }
    }
    popup.selectItem(at: choices.firstIndex(of: base) ?? 0)
    let filePopup = toolbar.filePopup
    filePopup.removeAllItems()
    for file in files { filePopup.addItem(withTitle: file.title) }
    if let index = currentFileIndex {
      filePopup.selectItem(at: index)
    } else {
      // The current file is not one of the listed files.
      let name = fileURL?.lastPathComponent ?? "Untitled"
      filePopup.addItem(withTitle: files.isEmpty ? name : "\(name) (not in this diff)")
      filePopup.selectItem(at: filePopup.numberOfItems - 1)
    }
    filePopup.isEnabled = !files.isEmpty
    toolbar.previousButton.isEnabled = !files.isEmpty
    toolbar.nextButton.isEnabled = !files.isEmpty
    toolbar.alignButton.state = isAligned ? .on : .off
    toolbar.previousChangeButton.isEnabled = session != nil
    toolbar.nextChangeButton.isEnabled = session != nil
  }
}
