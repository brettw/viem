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
  /// document's language. `reusing` is the previous file's base buffer,
  /// which is refilled in place so its surface and view survive.
  func makeDiffBase(source: Data, reusing: (any EVDocumentBackend)?) throws -> any EVDocumentBackend
  /// Compare `base` (shown by `baseSurface`) with this document (shown by
  /// `surface`). The session shows the document in Code until it ends.
  /// `editable` is false for a file deleted since the base. `collapsed`
  /// hides unchanged runs from the start, moving a caret out of them.
  func beginDiff(base: any EVDocumentBackend, baseSurface: any EVEditorSurface,
                 surface: any EVEditorSurface, aligned: Bool, collapsed: Bool,
                 editable: Bool) throws -> any EVDiffSessionControlling
}

extension EVDiffBackend {
  func makeDiffBase(source: Data) throws -> any EVDocumentBackend {
    try makeDiffBase(source: source, reusing: nil)
  }
}

/// One live comparison, owned by the window's diff mode.
@MainActor
public protocol EVDiffSessionControlling: AnyObject {
  var lineTotals: (added: UInt64, removed: UInt64) { get }
  var lineTotalsDidChange: (() -> Void)? { get set }
  func setAligned(_ aligned: Bool)
  /// Collapse unchanged context, or show it all; either closes every run
  /// the user opened.
  func setCollapsed(_ collapsed: Bool)
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
public enum EVDiffBase: Hashable, Sendable {
  case lastSaved
  case unstaged
  case head
  case branch(String)
  case commit(hash: String, short: String)
  case file(URL)

  var title: String {
    switch self {
    case .lastSaved: "Last Saved"
    case .unstaged: "Unstaged"
    case .head: "HEAD"
    case let .branch(name): "Branch: \(name)"
    case let .commit(hash, short): hash.isEmpty ? "Commit…" : "Commit \(short)"
    case let .file(url): "File: \(url.lastPathComponent)"
    }
  }

  var isGit: Bool {
    switch self {
    case .unstaged, .head, .branch, .commit: true
    case .lastSaved, .file: false
    }
  }

  nonisolated var kind: UInt32 {
    switch self {
    case .lastSaved: UInt32(VIEM_DIFF_BASE_LAST_SAVED)
    case .unstaged: UInt32(VIEM_DIFF_BASE_UNSTAGED)
    case .head: UInt32(VIEM_DIFF_BASE_HEAD)
    case .branch: UInt32(VIEM_DIFF_BASE_BRANCH)
    case .commit: UInt32(VIEM_DIFF_BASE_COMMIT)
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
  public var added: UInt64 = 0
  public var removed: UInt64 = 0

  var isDeleted: Bool { change == UInt32(VIEM_DIFF_FILE_DELETED) }

  func title(relativeTo directory: URL) -> String {
    let name = EVStatusFilePath.relative(url, relativeTo: directory)
    switch change {
    case UInt32(VIEM_DIFF_FILE_RENAMED): return "\(EVStatusFilePath.relative(baseURL, relativeTo: directory)) → \(name)"
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

  static func commits(path: String, hash: String? = nil) async throws -> [EVDiffCommit] {
    try await Task.detached(priority: .userInitiated) {
      let query = hash.map { EVDiffBase.commit(hash: $0, short: "") } ?? .head
      let data = try withResult(query, path: path, unsaved: [], call: viem_diff_query_commits) { handle, info in
        var bytes = Data(count: Int(info.byte_count))
        var required: UInt64 = 0
        let status = bytes.withUnsafeMutableBytes { buffer in
          viem_diff_result_copy_bytes(handle, buffer.bindMemory(to: UInt8.self).baseAddress, UInt64(buffer.count), &required)
        }
        return status == UInt32(VIEM_STATUS_OK) ? bytes : Data()
      }.get()
      return try JSONDecoder().decode([EVDiffCommit].self, from: data)
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
        return EVDiffFile(url: url, baseURL: baseURL, change: entry.change, added: entry.added, removed: entry.removed)
      }
    }
  }

  private nonisolated static func item(_ handle: ViemDiffResultHandle, _ index: UInt64) -> (path: String, basePath: String, change: UInt32, added: UInt64, removed: UInt64)? {
    var entry = ViemDiffListItem()
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
    return (path, basePath, entry.change, entry.added, entry.removed)
  }

  /// Build one request, call the core, read the owned result and release it.
  private nonisolated static func withResult<T>(
    _ base: EVDiffBase, path: String, unsaved: [String], scope: String? = nil,
    call: (UnsafePointer<ViemDiffBaseRequest>?, UnsafeMutablePointer<ViemDiffResultHandle>?) -> ViemStatus,
    read: (ViemDiffResultHandle, ViemDiffResultInfo) -> T
  ) -> Result<T, EVDiffLoadFailure> {
    let filePath: String
    let branch: String
    switch base {
    case let .file(url): filePath = url.path; branch = ""
    case let .branch(name), let .commit(name, _): filePath = path; branch = name
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
    var request = ViemDiffBaseRequest()
    request.kind = base.kind
    request.file_path = slice(0)
    request.revision = slice(1)
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
    var info = ViemDiffResultInfo()
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

/// Loaded bases by base and path. The base stays fixed while diff mode is
/// open (Refresh reloads it), so a loaded version stays valid until the
/// window's base, scope or Refresh discards the cache. Concurrent requests
/// for one version share a load, and the cache keeps a few recent versions
/// within a byte budget.
@MainActor
final class EVDiffBaseCache {
  struct Key: Hashable {
    var base: EVDiffBase
    var path: String
  }
  typealias Result = Swift.Result<EVDiffBaseLoader.Loaded, EVDiffLoadFailure>

  static let maximumEntries = 8
  static let maximumBytes = 64 << 20
  private var loaded: [Key: EVDiffBaseLoader.Loaded] = [:]
  private var recent: [Key] = []
  private var inFlight: [Key: Task<Result, Never>] = [:]
  /// Retires loads that finish after the cache was discarded.
  private var epoch = 0

  func cached(_ key: Key) -> EVDiffBaseLoader.Loaded? {
    guard let hit = loaded[key] else { return nil }
    recent.removeAll { $0 == key }
    recent.append(key)
    return hit
  }

  /// The version's load, started now unless it is cached or already running.
  @discardableResult
  func load(_ key: Key) -> Task<Result, Never> {
    if let hit = loaded[key] { return Task { .success(hit) } }
    if let running = inFlight[key] { return running }
    let epoch = self.epoch
    let task = Task { @MainActor [weak self] () -> Result in
      let result = await EVDiffBaseLoader.load(key.base, path: key.path)
      if let self, self.epoch == epoch {
        self.inFlight[key] = nil
        if case let .success(value) = result { self.store(value, for: key) }
      }
      return result
    }
    inFlight[key] = task
    return task
  }

  func removeAll() {
    epoch += 1
    loaded.removeAll()
    recent.removeAll()
    inFlight.removeAll()
  }

  func store(_ value: EVDiffBaseLoader.Loaded, for key: Key) {
    guard value.bytes.count <= Self.maximumBytes else { return }
    loaded[key] = value
    recent.removeAll { $0 == key }
    recent.append(key)
    while recent.count > Self.maximumEntries
            || loaded.values.reduce(0, { $0 + $1.bytes.count }) > Self.maximumBytes {
      loaded[recent.removeFirst()] = nil
    }
  }
}

/// The left (base) and right (document) halves of a diff window, separated
/// by the pane splitter. Dragging it moves the boundary; the base keeps its
/// share of the width while the window resizes.
@MainActor
final class EVDiffSplitView: NSView {
  /// The pane layout's splitter width and minimum pane width
  /// (`SPLITTER_WIDTH` and `MIN_WIDTH` in `src/core/layout/panes.rs`).
  static let splitterWidth: CGFloat = 5
  static let minimumWidth: CGFloat = 100

  let base: NSView
  let document: NSView
  let splitter = EVVerticalSplitter(frame: .zero)
  /// The base's share of the width beside the splitter.
  private(set) var baseFraction: CGFloat
  /// Reports each dragged share, so a replacement split can keep it.
  var baseFractionDidChange: ((CGFloat) -> Void)?
  /// Runs after each layout, so a status bar spanning both sides can follow.
  var didLayout: (() -> Void)?

  init(base: NSView, document: NSView, baseFraction: CGFloat = 0.5) {
    self.base = base
    self.document = document
    self.baseFraction = baseFraction
    super.init(frame: .zero)
    for view in [base, splitter, document] {
      view.translatesAutoresizingMaskIntoConstraints = true
      addSubview(view)
    }
    splitter.dragDidMove = { [weak self] delta in self?.moveSplitter(by: delta) }
    setAccessibilityElement(false)
  }

  @available(*, unavailable) required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  private var availableWidth: CGFloat { max(0, bounds.width - Self.splitterWidth) }

  /// `width` limited so that both sides keep their minimums. A window too
  /// narrow for both divides its width evenly.
  private func clampedBaseWidth(_ width: CGFloat) -> CGFloat {
    let available = availableWidth
    let upper = available - Self.minimumWidth
    guard upper >= Self.minimumWidth else { return available / 2 }
    return min(max(width, Self.minimumWidth), upper)
  }

  /// Like the pane splitter, a move past a minimum is discarded rather than
  /// accumulated, so reversing the drag moves the boundary at once.
  func moveSplitter(by delta: CGFloat) {
    let available = availableWidth
    guard available > 0 else { return }
    let width = clampedBaseWidth(clampedBaseWidth(available * baseFraction) + delta)
    baseFraction = width / available
    baseFractionDidChange?(baseFraction)
    needsLayout = true
    layoutSubtreeIfNeeded()
  }

  override func layout() {
    super.layout()
    let scale = window?.backingScaleFactor ?? 1
    let width = (clampedBaseWidth(availableWidth * baseFraction) * scale).rounded() / scale
    base.frame = NSRect(x: 0, y: 0, width: width, height: bounds.height)
    splitter.frame = NSRect(x: width, y: 0, width: Self.splitterWidth, height: bounds.height)
    document.frame = NSRect(x: splitter.frame.maxX, y: 0, width: max(0, bounds.width - splitter.frame.maxX),
                            height: bounds.height)
    didLayout?()
  }
}

/// The window's diff toolbar, shown below the title bar in diff mode.
@MainActor
final class EVDiffToolbar: NSObject {
  let accessory = NSTitlebarAccessoryViewController()
  let basePopup = NSPopUpButton(frame: .zero, pullsDown: false)
  let refreshButton = NSButton()
  let previousButton = NSButton()
  let filePopup = NSPopUpButton(frame: .zero, pullsDown: false)
  let fileCount = NSTextField(labelWithString: "")
  let fileCountGroup = NSStackView()
  let addedLines = NSTextField(labelWithString: "+0")
  let removedLines = NSTextField(labelWithString: "–0")
  let nextButton = NSButton()
  let alignButton = NSButton()
  let collapseButton = NSButton()
  let previousChangeButton = NSButton()
  let nextChangeButton = NSButton()
  let openWindowButton = NSButton()
  var selectBase: ((Int) -> Void)?
  var refresh: (() -> Void)?
  var previous: (() -> Void)?
  var next: (() -> Void)?
  var selectFile: ((Int) -> Void)?
  var toggleAlign: (() -> Void)?
  var toggleCollapse: (() -> Void)?
  var previousChange: (() -> Void)?
  var nextChange: (() -> Void)?
  var openWindow: (() -> Void)?

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

  static func fileIcon(openInWindow: Bool = false) -> NSImage {
    let image = NSImage(size: NSSize(width: 14, height: 17), flipped: true) { _ in
      NSColor.black.setStroke()
      let outline = NSBezierPath()
      outline.lineWidth = 1.3
      outline.move(to: NSPoint(x: 0.75, y: 6.75))
      outline.line(to: NSPoint(x: 6.75, y: 0.75))
      outline.line(to: NSPoint(x: 13.25, y: 0.75))
      outline.line(to: NSPoint(x: 13.25, y: 16.25))
      outline.line(to: NSPoint(x: 0.75, y: 16.25))
      outline.close()
      outline.move(to: NSPoint(x: 6.75, y: 0.75))
      outline.line(to: NSPoint(x: 6.75, y: 6.75))
      outline.line(to: NSPoint(x: 0.75, y: 6.75))
      outline.stroke()
      if openInWindow {
        let arrow = NSBezierPath()
        arrow.lineWidth = 2
        arrow.move(to: NSPoint(x: 3.7, y: 13.4))
        arrow.line(to: NSPoint(x: 10.6, y: 6.5))
        arrow.move(to: NSPoint(x: 7.3, y: 6.5))
        arrow.line(to: NSPoint(x: 10.6, y: 6.5))
        arrow.line(to: NSPoint(x: 10.6, y: 10.1))
        arrow.stroke()
      }
      return true
    }
    image.isTemplate = true
    return image
  }

  /// Two panes whose shared middle row crosses between them, as on Windows.
  static func alignIcon() -> NSImage {
    let image = NSImage(size: NSSize(width: 18, height: 16), flipped: true) { _ in
      NSColor.black.setStroke()
      let path = NSBezierPath()
      path.lineWidth = 1
      for x in [CGFloat(1.5), 10.5] {
        path.appendRect(NSRect(x: x, y: 2.5, width: 6, height: 11))
      }
      for (from, to, y) in [(CGFloat(3.5), CGFloat(5.5), CGFloat(5.5)), (12.5, 14.5, 5.5), (3.5, 14.5, 8),
                            (3.5, 5.5, 10.5), (12.5, 14.5, 10.5)] {
        path.move(to: NSPoint(x: from, y: y))
        path.line(to: NSPoint(x: to, y: y))
      }
      path.stroke()
      return true
    }
    image.isTemplate = true
    image.accessibilityDescription = "Align Sides"
    return image
  }

  /// Chevrons closing on a line: unchanged lines folded into a divider.
  static func collapseIcon() -> NSImage {
    let image = NSImage(size: NSSize(width: 16, height: 16), flipped: true) { _ in
      NSColor.black.setStroke()
      let path = NSBezierPath()
      path.lineWidth = 1.2
      path.lineCapStyle = .round
      path.lineJoinStyle = .round
      for (edge, tip) in [(CGFloat(1.5), CGFloat(5)), (14.5, 11)] {
        path.move(to: NSPoint(x: 4.5, y: edge))
        path.line(to: NSPoint(x: 8, y: tip))
        path.line(to: NSPoint(x: 11.5, y: edge))
      }
      path.move(to: NSPoint(x: 1.5, y: 8))
      path.line(to: NSPoint(x: 14.5, y: 8))
      path.stroke()
      return true
    }
    image.isTemplate = true
    image.accessibilityDescription = "Collapse Context"
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
    configure(refreshButton, symbol: "arrow.clockwise", label: "Refresh Base", action: #selector(refreshClicked(_:)))
    configure(previousButton, symbol: "chevron.left", label: "Previous File", action: #selector(previousClicked(_:)))
    configure(nextButton, symbol: "chevron.right", label: "Next File", action: #selector(nextClicked(_:)))
    previousButton.toolTip = "Previous File (:N)"
    nextButton.toolTip = "Next File (:n)"
    filePopup.target = self
    filePopup.action = #selector(fileChanged(_:))
    filePopup.toolTip = "Files in this diff"
    filePopup.setAccessibilityLabel("Files in this diff")
    configure(alignButton, symbol: "circle", label: "Align Sides", action: #selector(alignClicked(_:)))
    alignButton.setButtonType(.pushOnPushOff)
    alignButton.bezelStyle = .accessoryBar
    alignButton.image = Self.alignIcon()
    configure(collapseButton, symbol: "circle", label: "Collapse Context", action: #selector(collapseClicked(_:)))
    collapseButton.setButtonType(.pushOnPushOff)
    collapseButton.bezelStyle = .accessoryBar
    collapseButton.image = Self.collapseIcon()
    for (button, next) in [(previousChangeButton, false), (nextChangeButton, true)] {
      let label = next ? "Next Change" : "Previous Change"
      configure(button, symbol: "circle", label: label,
                action: next ? #selector(nextChangeClicked(_:)) : #selector(previousChangeClicked(_:)))
      button.image = Self.changeIcon(next: next)
    }
    filePopup.setContentHuggingPriority(.defaultLow, for: .horizontal)
    filePopup.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    filePopup.cell?.lineBreakMode = .byTruncatingHead
    configure(openWindowButton, symbol: "doc", label: "Open in New Editor Window", action: #selector(openWindowClicked(_:)))
    openWindowButton.image = Self.fileIcon(openInWindow: true)
    let fileIcon = NSImageView()
    fileIcon.image = Self.fileIcon()
    fileCountGroup.orientation = .horizontal
    fileCountGroup.isHidden = true
    fileCountGroup.spacing = 3
    fileCountGroup.addArrangedSubview(fileIcon)
    fileCountGroup.addArrangedSubview(fileCount)
    addedLines.textColor = .systemGreen
    removedLines.textColor = .systemRed
    let preferredWidth = filePopup.widthAnchor.constraint(greaterThanOrEqualToConstant: 220)
    preferredWidth.priority = .defaultLow
    preferredWidth.isActive = true
    for view in [basePopup, refreshButton, fileCountGroup, addedLines, removedLines, previousButton, nextButton, filePopup, alignButton,
                 collapseButton, previousChangeButton, nextChangeButton] as [NSView] {
      stack.addArrangedSubview(view)
    }
    stack.setCustomSpacing(3 * stack.spacing, after: refreshButton)
    stack.setCustomSpacing(3 * stack.spacing, after: filePopup)
    let container = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 34))
    stack.translatesAutoresizingMaskIntoConstraints = false
    openWindowButton.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(stack)
    container.addSubview(openWindowButton)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      stack.topAnchor.constraint(equalTo: container.topAnchor),
      stack.bottomAnchor.constraint(equalTo: container.bottomAnchor),
      stack.trailingAnchor.constraint(equalTo: openWindowButton.leadingAnchor),
      openWindowButton.centerYAnchor.constraint(equalTo: stack.centerYAnchor),
      openWindowButton.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -10),
    ])
    accessory.view = container
  }

  @objc private func baseChanged(_ sender: NSPopUpButton) { selectBase?(sender.indexOfSelectedItem) }
  @objc private func refreshClicked(_ sender: Any?) { refresh?() }
  @objc private func previousClicked(_ sender: Any?) { previous?() }
  @objc private func nextClicked(_ sender: Any?) { next?() }
  @objc private func fileChanged(_ sender: NSPopUpButton) {
    if let index = sender.selectedItem?.representedObject as? Int { selectFile?(index) }
  }
  @objc private func alignClicked(_ sender: Any?) { toggleAlign?() }
  @objc private func collapseClicked(_ sender: Any?) { toggleCollapse?() }
  @objc private func previousChangeClicked(_ sender: Any?) { previousChange?() }
  @objc private func nextChangeClicked(_ sender: Any?) { nextChange?() }
  @objc private func openWindowClicked(_ sender: Any?) { openWindow?() }
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
  /// Every pane shows one side-by-side pair in diff mode.
  var diffPanes: [EVDocumentContentViewController] { get }
  var diffActivePane: EVDocumentContentViewController? { get }
  var diffOpenPanelPresenter: ((NSOpenPanel, NSWindow, @escaping @MainActor (URL?) -> Void) -> Void)? { get }
  /// Replace the active pane's document with `url`, discarding unsaved
  /// changes the caller has already reviewed.
  func openDiffFile(_ url: URL, beforeReplacing: @escaping @MainActor () -> Bool,
                    completion: @escaping @MainActor (Error?) -> Void)
  /// Unsaved open documents, for the file list.
  func unsavedDocumentURLs() -> [URL]
}

/// One pane's comparison: the base buffer beside the pane's editor and the
/// core session between them. Splitting a pane splits the pair.
@MainActor
private final class EVDiffPair {
  weak var pane: EVDocumentContentViewController?
  /// The document and editor the pair last presented.
  weak var document: EVDocument?
  weak var surface: AnyObject?
  var baseBackend: (any EVDocumentBackend)?
  var baseSurface: (any EVEditorSurface)?
  var session: (any EVDiffSessionControlling)?
  /// Retires base loads a newer request for this pane superseded.
  var generation = 0

  init(pane: EVDocumentContentViewController) { self.pane = pane }

  var isPresentingPane: Bool {
    guard let pane else { return false }
    return pane.document === document && pane.editorSurface as AnyObject === surface
  }
}

/// One window's diff mode: its base, file list, toolbar and comparisons.
@MainActor
final class EVDiffModeController: NSObject {
  private weak var host: (any EVDiffModeHost)?
  private let configuration: EVConfigurationStore
  let toolbar = EVDiffToolbar()
  private(set) var isEnabled = false
  private(set) var base: EVDiffBase = .lastSaved
  private(set) var files: [EVDiffFile] = [] {
    didSet {
      gitTotals = files.reduce(into: (added: UInt64(0), removed: UInt64(0))) { total, file in
        total.added += file.added
        total.removed += file.removed
      }
    }
  }
  private var gitTotals: (added: UInt64, removed: UInt64) = (0, 0)
  private(set) var branches: [String] = []
  private(set) var isAligned: Bool
  private(set) var isCollapsed: Bool
  /// The folder a folder diff lists; nil lists the whole repository.
  private(set) var scope: URL?
  private var singleFile = false
  private var hasRepository = false
  private var commitPickerTask: Task<Void, Never>?
  private var chosenFileLoad: Task<Void, Never>?
  private var baseChoices: [EVDiffBase] = []
  private var pairs: [EVDiffPair] = []
  /// A replaced pane's base buffer and view, which the next pair refills so
  /// moving between files keeps the base view.
  private var spareBase: (backend: any EVDocumentBackend, surface: any EVEditorSurface)?
  /// The base's share of each pair, kept while the window lives.
  private(set) var splitFraction: CGFloat = 0.5
  private let baseCache = EVDiffBaseCache()
  /// The document the toolbar describes.
  private weak var toolbarDocument: EVDocument?
  /// Retires file listings that a newer request superseded.
  private var generation = 0
  /// Layout during an install can report an active-document change; the
  /// pair being installed already presents its pane.
  private var isInstalling = false
  /// Pairs wait while diff mode starts and chooses its base.
  private var isChoosingBase = false
  private(set) var lastMessage: String?
  /// Tests await this after a change to know the async work is installed.
  var didPresent: (() -> Void)?

  init(host: any EVDiffModeHost, configuration: EVConfigurationStore) {
    self.host = host
    self.configuration = configuration
    isAligned = configuration.diffAlignSides
    isCollapsed = configuration.diffCollapseContext
    super.init()
    toolbar.selectBase = { [weak self] index in self?.selectBase(at: index) }
    toolbar.refresh = { [weak self] in self?.refresh() }
    toolbar.previous = { [weak self] in self?.moveFile(by: -1) }
    toolbar.next = { [weak self] in self?.moveFile(by: 1) }
    toolbar.selectFile = { [weak self] index in self?.selectFile(at: index) }
    toolbar.toggleAlign = { [weak self] in self?.toggleAlign() }
    toolbar.toggleCollapse = { [weak self] in self?.toggleCollapse() }
    toolbar.previousChange = { [weak self] in self?.navigateChange(next: false) }
    toolbar.nextChange = { [weak self] in self?.navigateChange(next: true) }
    toolbar.openWindow = { [weak self] in self?.activeDocument?.showAdditionalWindow() }
  }

  private var activeDocument: EVDocument? { host?.diffActivePane?.document }
  private var activePair: EVDiffPair? {
    guard let pane = host?.diffActivePane else { return nil }
    return pairs.first { $0.pane === pane }
  }

  /// Both sides of every pair, for settings diff mode applies to all of them.
  private var pairSurfaces: [any EVEditorSurface] {
    pairs.flatMap { pair in [pair.pane?.editorSurface, pair.baseSurface].compactMap { $0 } }
  }

  // MARK: Entering and leaving

  func toggle() {
    if isEnabled { disable() } else { enable() }
  }

  /// Turn diff mode on and show the toolbar; false when it cannot start.
  private func begin() -> Bool {
    guard !isEnabled, let host, let window = host.diffWindow, activeDocument != nil else { return false }
    isChoosingBase = true
    isEnabled = true
    isAligned = configuration.diffAlignSides
    isCollapsed = configuration.diffCollapseContext
    if !window.titlebarAccessoryViewControllers.contains(where: { $0 === toolbar.accessory }) {
      window.addTitlebarAccessoryViewController(toolbar.accessory)
    }
    return true
  }

  /// Compare the files `git diff -- <folder>` lists, starting with the
  /// active document (normally the first of `files`).
  func enable(folder: URL, files: [EVDiffFile]) {
    guard isEnabled || begin() else { return }
    baseCache.removeAll()
    scope = folder
    singleFile = false
    hasRepository = true
    base = .unstaged
    self.files = files
    generation += 1
    isChoosingBase = false
    present()
  }

  func enable(singleFile: Bool = false) {
    guard begin() else { return }
    self.singleFile = singleFile
    hasRepository = false
    scope = nil
    let url = activeDocument?.fileURL
    generation += 1
    let current = generation
    // Default to `git diff`, falling back to Last saved outside a repository.
    guard let url, let document = activeDocument else {
      base = .lastSaved
      isChoosingBase = false
      present()
      return
    }
    Task { @MainActor in
      let listed = await EVDiffBaseLoader.listFiles(.unstaged, path: url.path, unsaved: self.unsavedPaths())
      guard current == self.generation, self.isEnabled else { return }
      guard self.activeDocument === document else { self.disable(); return }
      switch listed {
      case let .success(files):
        self.hasRepository = true
        self.selectListedBase(.unstaged, files: singleFile ? files.filter { $0.url.standardizedFileURL == url.standardizedFileURL } : files,
                              for: document)
      case .failure:
        self.base = .lastSaved
        self.files = [EVDiffFile(url: url, baseURL: url, change: UInt32(VIEM_DIFF_FILE_MODIFIED))]
        self.isChoosingBase = false
        self.present()
      }
    }
  }

  func disable() {
    guard isEnabled else { return }
    isEnabled = false
    commitPickerTask?.cancel()
    chosenFileLoad?.cancel()
    singleFile = false
    isChoosingBase = false
    generation += 1
    let panes = pairs.compactMap(\.pane)
    for pair in pairs { end(pair) }
    pairs = []
    spareBase = nil
    baseCache.removeAll()
    if let code = configuration.codeLineNumbers {
      for pane in panes { (pane.editorSurface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(code) }
    }
    if let window = host?.diffWindow,
       let index = window.titlebarAccessoryViewControllers.firstIndex(where: { $0 === toolbar.accessory }) {
      window.removeTitlebarAccessoryViewController(at: index)
    }
    files = []
    scope = nil
    toolbarDocument = nil
  }

  /// End a pair's comparison and remove its base from the pane. A pane that
  /// is closing leaves its base buffer and view for the next pair to refill.
  private func end(_ pair: EVDiffPair, keepingBase: Bool = false) {
    pair.generation += 1
    pair.session?.end()
    pair.session = nil
    pair.pane?.detachDiffBase()
    if keepingBase, let backend = pair.baseBackend, let surface = pair.baseSurface {
      spareBase = (backend, surface)
    }
    pair.baseBackend = nil
    pair.baseSurface = nil
  }

  /// Panes were added, removed or replaced, or a pane's document changed by
  /// another route (`:e`, a link, a split): compare every pane with the same
  /// base, each in its own pair.
  func panesDidChange() {
    guard isEnabled, !isInstalling, !isChoosingBase else { return }
    syncPairs()
    for pair in pairs where !pair.isPresentingPane { present(pair) }
    if toolbarDocument !== activeDocument { refreshToolbar() }
  }

  /// One pair per pane, in pane order; a closed pane's base is kept spare.
  private func syncPairs() {
    guard let host else { return }
    let panes = host.diffPanes.filter { $0.document != nil }
    for pair in pairs where !panes.contains(where: { $0 === pair.pane }) { end(pair, keepingBase: true) }
    pairs = panes.map { pane in pairs.first { $0.pane === pane } ?? EVDiffPair(pane: pane) }
  }

  /// The active pane or its document changed.
  func activeDocumentDidChange() { panesDidChange() }

  // MARK: Presenting

  private func basePath(for document: EVDocument) -> String? {
    guard let url = document.fileURL else { return nil }
    return files.first(where: { $0.url.standardizedFileURL == url.standardizedFileURL })?.baseURL.path ?? url.path
  }

  /// "BASE @ <version>" names where a pair's base came from.
  static func baseStatusTitle(_ base: EVDiffBase) -> String {
    switch base {
    case .lastSaved: "BASE @ saved"
    case .unstaged: "BASE @ index"
    case .head: "BASE @ HEAD"
    case let .branch(name): "BASE @ \(name)"
    case let .commit(_, short): "BASE @ \(short)"
    case .file: "BASE"
    }
  }

  /// The base's file where it differs from the document's: a chosen file, or
  /// a renamed file's original name.
  private func baseStatusFileURL(for document: EVDocument, base: EVDiffBase) -> URL? {
    if case let .file(url) = base { return url }
    guard let url = document.fileURL?.standardizedFileURL,
          let file = files.first(where: { $0.url.standardizedFileURL == url }),
          file.baseURL.standardizedFileURL != url else { return nil }
    return file.baseURL
  }

  /// Load the base for every pane's document and show the comparisons.
  func present(readingDisk: Bool = false, loadedBase: Data? = nil, forFile file: URL? = nil) {
    guard isEnabled, !isChoosingBase else { return }
    if readingDisk { baseCache.removeAll() }
    syncPairs()
    refreshToolbar()
    for pair in pairs {
      let matches = file == nil || pair.pane?.document?.fileURL?.standardizedFileURL == file?.standardizedFileURL
      present(pair, readingDisk: readingDisk, loadedBase: matches ? loadedBase : nil)
    }
  }

  private func present(_ pair: EVDiffPair, readingDisk: Bool = false, loadedBase: Data? = nil) {
    guard isEnabled, let pane = pair.pane, let document = pane.document else { return }
    pair.generation += 1
    let current = pair.generation
    pair.document = document
    pair.surface = pane.editorSurface as AnyObject
    let base = self.base
    let deleted = files.contains { $0.isDeleted && $0.url.path == document.fileURL?.path }
    if let loadedBase {
      install(base: loadedBase, from: base, in: pair, editable: !deleted)
      return
    }
    if base == .lastSaved, !readingDisk,
       let saved = (document.editorBackend as? any EVDiffBackend)?.diffSavedSource() {
      install(base: saved, from: base, in: pair)
      return
    }
    let path: String
    if case let .file(url) = base {
      path = url.path
    } else if let documentPath = basePath(for: document) {
      path = documentPath
    } else {
      // An untitled document's Last saved base is empty.
      install(base: Data(), from: base, in: pair)
      return
    }
    let key = EVDiffBaseCache.Key(base: base, path: path)
    if let cached = baseCache.cached(key) {
      install(base: cached.bytes, from: base, in: pair, editable: !deleted)
      return
    }
    let loading = baseCache.load(key)
    Task { @MainActor in
      let loaded = await loading.value
      guard current == pair.generation, self.isEnabled, self.pairs.contains(where: { $0 === pair }),
            pair.pane?.document === document else { return }
      switch loaded {
      case let .success(result): self.install(base: result.bytes, from: base, in: pair, editable: !deleted)
      case let .failure(error):
        self.lastMessage = error.localizedDescription
        pair.pane?.editorSurface.showDocumentMessage(error.localizedDescription)
        self.install(base: Data(), from: base, in: pair, editable: !deleted)
      }
    }
  }

  private func install(base bytes: Data, from version: EVDiffBase, in pair: EVDiffPair, editable: Bool = true) {
    guard !isInstalling, let pane = pair.pane, let document = pane.document,
          let backend = document.editorBackend as? any EVDiffBackend else { return }
    let surface = pane.editorSurface
    isInstalling = true
    defer { isInstalling = false }
    // Refilling a previous base buffer keeps its view, so the pane's layout
    // does not change between files.
    pair.session?.end()
    pair.session = nil
    let previous = pair.baseBackend.map { ($0, pair.baseSurface) } ?? spareBase.map { ($0.backend, Optional($0.surface)) }
    if pair.baseBackend == nil { spareBase = nil }
    do {
      let baseBackend: any EVDocumentBackend
      if let reusing = previous?.0, let made = try? backend.makeDiffBase(source: bytes, reusing: reusing) {
        baseBackend = made
      } else {
        baseBackend = try backend.makeDiffBase(source: bytes, reusing: nil)
      }
      let reused = previous?.0 === baseBackend
      let baseSurface = (reused ? previous?.1 : nil) ?? baseBackend.makeEditorSurface()
      pair.baseBackend = baseBackend
      pair.baseSurface = baseSurface
      pane.attachDiffBase(baseSurface, baseFraction: splitFraction) { [weak self] fraction in
        self?.splitFraction = fraction
      }
      pane.setDiffBaseStatus(title: Self.baseStatusTitle(version), fileURL: baseStatusFileURL(for: document, base: version))
      pane.view.layoutSubtreeIfNeeded()
      pair.session = try backend.beginDiff(base: baseBackend, baseSurface: baseSurface, surface: surface,
                                           aligned: isAligned, collapsed: isCollapsed, editable: editable)
      pair.session?.lineTotalsDidChange = { [weak self, weak pair] in
        guard let self, let pair, pair === self.activePair, !self.base.isGit || self.singleFile else { return }
        self.refreshStatistics()
      }
      let numbers = configuration.diffLineNumbers
      (baseSurface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(numbers)
      (surface as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(numbers)
      lastMessage = nil
    } catch {
      lastMessage = error.localizedDescription
      surface.showDocumentMessage(error.localizedDescription)
    }
    if pane === host?.diffActivePane { refreshToolbar() }
    prefetchNeighbours()
    didPresent?()
  }

  /// Start loading the bases of the files the arrows reach next, so moving
  /// through the list finds them ready.
  private func prefetchNeighbours() {
    guard base.isGit || { if case .file = base { return true } else { return false } }(),
          files.count > 1, let index = currentFileIndex else { return }
    for step in [1, -1] {
      prefetch(files[((index + step) % files.count + files.count) % files.count])
    }
  }

  private func prefetch(_ file: EVDiffFile) {
    let path: String
    switch base {
    case let .file(url): path = url.path
    case .lastSaved: return
    default: path = file.baseURL.path
    }
    baseCache.load(EVDiffBaseCache.Key(base: base, path: path))
  }

  // MARK: Toolbar and menu actions

  func refresh() {
    guard isEnabled, let document = activeDocument else { return }
    generation += 1
    baseCache.removeAll()
    for pair in pairs { pair.generation += 1 }
    listFiles(for: base, document: document) { files in
      self.files = files
      self.present(readingDisk: true)
    }
  }

  private func listFiles(for base: EVDiffBase, document: EVDocument, then: @escaping @MainActor ([EVDiffFile]) -> Void) {
    let url = document.fileURL
    switch base {
    case .file:
      then([])
    case .lastSaved:
      then(url.map { [EVDiffFile(url: $0, baseURL: $0, change: UInt32(VIEM_DIFF_FILE_MODIFIED))] } ?? [])
    default:
      // A folder diff's empty buffer finds the repository from its folder.
      guard let path = url?.path ?? scope?.path else { then([]); return }
      let current = generation
      Task { @MainActor in
        let listed = await EVDiffBaseLoader.listFiles(base, path: path, unsaved: self.unsavedPaths(),
                                                      scope: self.scope?.path)
        guard current == self.generation, self.isEnabled, self.activeDocument === document else { return }
        switch listed {
        case let .success(files):
          then(self.singleFile ? files.filter { $0.url.standardizedFileURL == url?.standardizedFileURL } : files)
        case let .failure(error):
          self.lastMessage = error.localizedDescription
          self.host?.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription)
          self.refreshToolbar()
        }
      }
    }
  }

  func setBase(_ next: EVDiffBase) {
    guard isEnabled, let document = activeDocument else { return }
    chosenFileLoad?.cancel()
    generation += 1
    refreshToolbar() // Keep the displayed choice until the change succeeds.
    if case let .file(url) = next {
      let current = generation
      chosenFileLoad = Task { @MainActor in
        let result = await EVDiffBaseLoader.load(next, path: url.path)
        guard !Task.isCancelled, current == self.generation, self.isEnabled, self.activeDocument === document else { return }
        switch result {
        case let .success(loaded):
          self.baseCache.removeAll()
          self.baseCache.store(loaded, for: .init(base: next, path: url.path))
          self.base = next
          self.scope = nil
          self.singleFile = true
          self.files = []
          self.isChoosingBase = false
          if document.fileURL == nil { self.hasRepository = false }
          self.present(loadedBase: loaded.bytes)
        case let .failure(error):
          self.host?.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription)
        }
      }
      return
    }
    listFiles(for: next, document: document) { files in
      self.selectListedBase(next, files: files, for: document)
    }
  }

  /// A base and its first file install together after any save review and
  /// file open. Until then the toolbar and panes describe the previous base.
  private func selectListedBase(_ next: EVDiffBase, files: [EVDiffFile], for document: EVDocument) {
    func installSelection() {
      baseCache.removeAll()
      base = next
      self.files = files
    }
    guard let first = files.first,
          !files.contains(where: { $0.url.standardizedFileURL == document.fileURL?.standardizedFileURL }) else {
      installSelection()
      isChoosingBase = false
      present()
      return
    }
    let current = generation
    let pane = host?.diffActivePane
    func isCurrent() -> Bool {
      isEnabled && generation == current && host?.diffActivePane === pane && activeDocument === document
    }
    func cancel(_ error: Error? = nil) {
      guard isEnabled, generation == current else { return }
      if isChoosingBase { disable() } else { refreshToolbar() }
      if let error { host?.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription) }
    }
    reviewUnsavedChanges(of: document) { proceed in
      guard proceed, isCurrent() else { cancel(); return }
      let reviewed = document.editorBackend.persistenceState
      Task { @MainActor in
        let result = await EVDiffBaseLoader.load(next, path: first.baseURL.path)
        guard isCurrent(), document.editorBackend.persistenceState == reviewed else { cancel(); return }
        switch result {
        case let .failure(error): cancel(error)
        case let .success(loaded):
          self.host?.openDiffFile(first.url, beforeReplacing: {
            guard isCurrent() else { return false }
            installSelection()
            self.baseCache.store(loaded, for: .init(base: next, path: first.baseURL.path))
            self.isChoosingBase = true // Replace the pane before presenting its new comparison.
            return true
          }) { error in
            if let error { cancel(error); return }
            self.isChoosingBase = false
            self.present(loadedBase: loaded.bytes, forFile: first.url)
          }
        }
      }
    }
  }

  private func selectBase(at index: Int) {
    if index == baseChoices.count { chooseBaseFile(); return }
    guard baseChoices.indices.contains(index) else { return }
    let choice = baseChoices[index]
    if case .branch("") = choice {
      chooseBranch()
    } else if case .commit("", _) = choice {
      chooseCommit()
    } else {
      setBase(choice)
    }
  }

  private func chooseCommit() {
    refreshToolbar()
    guard let document = activeDocument, let path = scope?.path ?? document.fileURL?.path,
          let window = host?.diffWindow else { return }
    commitPickerTask?.cancel()
    let current = generation
    commitPickerTask = Task { @MainActor in
      @MainActor func isCurrent() -> Bool {
        !Task.isCancelled && self.isEnabled && self.generation == current && self.activeDocument === document
      }
      do {
        let commits = try await EVDiffBaseLoader.commits(path: path)
        guard isCurrent() else { return }
        let picker = EVDiffCommitPicker(commits)
        if case let .commit(hash, _) = self.base { picker.input.stringValue = hash }
        while isCurrent(), let hash = await picker.pick(in: window) {
          guard isCurrent() else { return }
          do {
            let selected: EVDiffCommit?
            if let listed = commits.first(where: { $0.hash.caseInsensitiveCompare(hash) == .orderedSame }) {
              selected = listed
            } else {
              selected = try await EVDiffBaseLoader.commits(path: path, hash: hash).first
            }
            guard isCurrent(), let selected else { return }
            self.setBase(.commit(hash: selected.hash, short: selected.short))
            return
          } catch { picker.alert.informativeText = error.localizedDescription }
        }
      } catch {
        if isCurrent() { self.host?.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription) }
      }
    }
  }

  private func chooseBranch() {
    guard let path = scope?.path ?? activeDocument?.fileURL?.path else { return }
    let current = generation
    let document = activeDocument
    Task { @MainActor in
      let listed = await EVDiffBaseLoader.listBranches(path: path)
      guard self.isEnabled, self.generation == current, self.activeDocument === document else { return }
      switch listed {
      case let .success(branches):
        self.branches = branches
        self.refreshToolbar()
        self.toolbar.basePopup.performClick(nil)
      case let .failure(error):
        self.host?.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription)
        self.refreshToolbar()
      }
    }
  }

  func chooseBaseFile() {
    guard isEnabled, let host, let window = host.diffWindow, let document = activeDocument else { return }
    refreshToolbar()
    let current = generation
    let panel = NSOpenPanel()
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    // Every file is a valid base, including extensionless files and dotfiles.
    panel.allowedContentTypes = []
    panel.showsHiddenFiles = true
    panel.message = "Choose a file to compare with"
    let selected: @MainActor (URL?) -> Void = { [weak self] url in
      guard let self, let url, self.isEnabled, self.generation == current, self.activeDocument === document else { return }
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
    for pair in pairs { pair.session?.setAligned(isAligned) }
    refreshToolbar()
  }

  /// Collapse context, or show it, in every pair; saved for new diff views.
  func toggleCollapse() {
    isCollapsed.toggle()
    try? configuration.setDiffCollapseContext(isCollapsed)
    for pair in pairs { pair.session?.setCollapsed(isCollapsed) }
    refreshToolbar()
  }

  /// Move both sides of the active pair to the next or previous change.
  func navigateChange(next: Bool) {
    guard isEnabled else { return }
    activePair?.session?.navigateChange(next: next)
  }

  /// A line-number toggle on either side of any pair applies to every pair
  /// and is saved.
  func lineNumbersDidToggle(on surface: any EVEditorSurface) {
    guard isEnabled, let shows = (surface as? any EVDiffLineNumberControlling)?.showsLineNumbers else { return }
    try? configuration.setDiffLineNumbers(shows)
    for side in pairSurfaces { (side as? any EVDiffLineNumberControlling)?.setShowsLineNumbers(shows) }
  }

  var currentFileIndex: Int? {
    guard let url = activeDocument?.fileURL?.standardizedFileURL else { return nil }
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
    guard isEnabled, files.indices.contains(index), let host, let document = activeDocument else { return }
    let file = files[index]
    let target = file.url
    guard target.standardizedFileURL != document.fileURL?.standardizedFileURL else { return }
    generation += 1
    let current = generation
    func isCurrent() -> Bool { isEnabled && generation == current && activeDocument === document }
    reviewUnsavedChanges(of: document) { [weak self] proceed in
      guard let self, proceed, isCurrent() else { self?.refreshToolbar(); return }
      // Load the base while the document opens.
      self.prefetch(file)
      host.openDiffFile(target, beforeReplacing: isCurrent) { error in
        if let error { host.diffActivePane?.editorSurface.showDocumentMessage(error.localizedDescription) }
        self.panesDidChange()
        self.refreshToolbar()
      }
    }
  }

  /// The save prompt a window close shows: Save, Don't Save or Cancel.
  private var pendingReview: ((Bool) -> Void)?
  private func reviewUnsavedChanges(of document: EVDocument, completion: @escaping (Bool) -> Void) {
    guard pendingReview == nil else { completion(false); return }
    guard document.editorBackend.persistenceState.isDirty || document.isDocumentEdited else {
      completion(true)
      return
    }
    let reviewed = document.editorBackend.persistenceState
    pendingReview = { approved in
      let state = document.editorBackend.persistenceState
      let saved = !state.isDirty && !document.isDocumentEdited
      completion(approved && (saved || (state.documentID == reviewed.documentID && state.documentRevision == reviewed.documentRevision)))
    }
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

  /// The file popup's entry for a folder without changes.
  static let noChangesTitle = "no changes"

  private func refreshStatistics() {
    let useLiveTotals = !base.isGit || singleFile && (files.isEmpty || activeDocument?.editorBackend.persistenceState.isDirty == true)
    let totals = useLiveTotals ? (activePair?.session?.lineTotals ?? (0, 0)) : gitTotals
    toolbar.addedLines.stringValue = "+\(totals.0)"
    toolbar.removedLines.stringValue = "–\(totals.1)"
    toolbar.addedLines.setAccessibilityLabel("\(totals.0) added lines")
    toolbar.removedLines.setAccessibilityLabel("\(totals.1) removed lines")
  }

  func refreshToolbar() {
    refreshStatistics()
    toolbar.fileCountGroup.isHidden = files.count <= 1 && !(base.isGit && scope != nil && files.isEmpty)
    toolbar.fileCount.stringValue = String(files.count)
    toolbar.fileCount.setAccessibilityLabel("\(files.count) changed files")
    toolbarDocument = activeDocument
    let fileURL = activeDocument?.fileURL
    var choices: [EVDiffBase] = [.lastSaved]
    if hasRepository { choices += [.unstaged, .head] }
    choices += branches.map { .branch($0) }
    if hasRepository { choices.append(.branch("")) }
    if case .file = base { choices.append(base) }
    if case .branch = base, !choices.contains(base) { choices.insert(base, at: min(3, choices.count)) }
    if case .commit = base { choices.append(base) }
    if hasRepository { choices.append(.commit(hash: "", short: "")) }
    baseChoices = choices
    // NSPopUpButton.addItem(withTitle:) replaces an item with the same
    // title, which would shift every later index; add menu items directly so
    // item i is always choice i (or file i).
    func fill(_ popup: NSPopUpButton, with titles: [String]) {
      popup.removeAllItems()
      for title in titles { popup.menu?.addItem(NSMenuItem(title: title, action: nil, keyEquivalent: "")) }
    }
    let popup = toolbar.basePopup
    fill(popup, with: choices.map { if case .branch("") = $0 { "Branch…" } else { $0.title } })
    popup.addItem(withTitle: "Choose File…")
    popup.selectItem(at: choices.firstIndex(of: base) ?? 0)
    let filePopup = toolbar.filePopup
    let current = currentFileIndex
    let limit = Int(VIEM_DIFF_FILE_PICKER_LIMIT)
    var indices = Array(files.indices.prefix(limit))
    if let current, current >= limit { indices[limit - 1] = current }
    let directory = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
    fill(filePopup, with: indices.map { files[$0].title(relativeTo: directory) })
    for (slot, index) in indices.enumerated() { filePopup.item(at: slot)?.representedObject = index }
    if let current {
      filePopup.selectItem(at: min(current, limit - 1))
    } else if files.isEmpty, scope != nil {
      // A folder without changes shows an empty buffer.
      let item = NSMenuItem(title: Self.noChangesTitle, action: nil, keyEquivalent: "")
      let font = filePopup.font ?? .systemFont(ofSize: NSFont.systemFontSize)
      item.attributedTitle = NSAttributedString(string: Self.noChangesTitle,
        attributes: [.font: NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)])
      filePopup.menu?.addItem(item)
      filePopup.selectItem(at: filePopup.numberOfItems - 1)
    } else {
      // The current file is not one of the listed files.
      let name = fileURL.map { EVStatusFilePath.relative($0, relativeTo: directory) } ?? "Untitled"
      filePopup.menu?.addItem(NSMenuItem(title: files.isEmpty ? name : "\(name) (not in this diff)", action: nil, keyEquivalent: ""))
      filePopup.selectItem(at: filePopup.numberOfItems - 1)
    }
    if files.count > limit {
      filePopup.menu?.autoenablesItems = false
      let footer = NSMenuItem(title: "\(files.count - limit) files clipped", action: nil, keyEquivalent: "")
      footer.isEnabled = false
      filePopup.menu?.addItem(footer)
    }
    filePopup.isEnabled = !files.isEmpty
    filePopup.toolTip = filePopup.titleOfSelectedItem
    toolbar.previousButton.isEnabled = !files.isEmpty
    toolbar.nextButton.isEnabled = !files.isEmpty
    toolbar.alignButton.state = isAligned ? .on : .off
    toolbar.collapseButton.state = isCollapsed ? .on : .off
    toolbar.previousChangeButton.isEnabled = activePair?.session != nil
    toolbar.nextChangeButton.isEnabled = activePair?.session != nil
  }
}
