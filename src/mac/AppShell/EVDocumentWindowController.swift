import AppKit
import CViemCore

/// AppKit may ask a window to constrain itself before it has been assigned a
/// screen during direct SwiftPM application startup. NSWindow's default
/// no-screen result collapses the requested frame to its fitting/minimum size.
/// Preserve the requested frame until a real screen exists, then use AppKit's
/// normal constraints for native moving, tiling, zooming, and resizing.
private final class EVDocumentWindow: NSWindow {
  override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
    guard let screen else { return frameRect }
    return super.constrainFrameRect(frameRect, to: screen)
  }
}

@MainActor
public final class EVDocumentWindowController: NSWindowController, EVDocumentHostEffectHandling,
  NSWindowDelegate
{
  static let initialContentSize = NSSize(width: 920, height: 680)
  static let minimumContentSize = NSSize(width: 480, height: 280)

  public var editorSurface: any EVEditorSurface { paneContainer.activePane.editorSurface }
  var documentContentController: EVDocumentContentViewController { paneContainer.activePane }
  public var activeDocument: EVDocument? { paneContainer.activePane.document }
  public func documentURL(for surface: any EVEditorSurface) -> URL? {
    paneContainer.panes.first(where: { $0.editorSurface === surface })?.document?.fileURL
  }
  private(set) var paneContainer: EVPaneContainer
  private static var instances: [EVWeakDocumentWindow] = []
  static var hasOpenDocumentWindows: Bool {
    instances.contains { $0.value.map { !$0.isClosed } ?? false }
  }
  static func hasOpenViews(of document: EVDocument) -> Bool {
    instances.compactMap(\.value).contains { controller in
      !controller.isClosed && controller.paneContainer.panes.contains { $0.document === document }
    }
  }
  var terminateWithStatus: (Int32) -> Void = { exit($0) }
  var commandDidCloseWindow: () -> Void = {
    (NSApplication.shared.delegate as? EVApplicationDelegate)?.terminateAfterCommandClose()
  }
  private var isClosed = false
  private var closeQueue: [EVDocument] = []
  private var closeReviewCompletion: ((Bool) -> Void)?
  private var pendingPaneClose: (() -> Void)?
  private var closingAfterReview = false
  private var hasPresentedInitialWindow = false
  private var isPerformingDocumentHostEffect = false
  private let placement: EVDocumentWindowPlacement
  /// Where the window's frame comes from and is remembered while it shows a
  /// diff; regular windows use `placement`.
  private let diffPlacement: EVDocumentWindowPlacement
  /// A window opened to show a diff starts from the last diff window's frame.
  var opensForDiff = false
  private var activePlacement: EVDocumentWindowPlacement {
    diffMode?.isEnabled == true || (opensForDiff && diffMode == nil) ? diffPlacement : placement
  }
  private let configuration: EVConfigurationStore
  private var initialWindowFrame: NSRect?
  private(set) lazy var formattingToolbar = EVFormattingToolbarChrome(
    window: window!, configuration: configuration)
  /// Created when diff mode is first used in this window.
  private(set) var diffMode: EVDiffModeController?

  /// Injectable I/O boundary for launch and argument-list opens.
  var argumentDocumentOpener: ((URL, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void)?

  /// Tests can complete a configured panel without starting AppKit's modal loop.
  var openFilePanelPresenter: ((NSOpenPanel, NSWindow, @escaping @MainActor (URL?) -> Void) -> Void)?

  var currentGeometry: EVDocumentWindowGeometry? {
    guard let window, synchronizeContentFrame(of: window) != nil else { return nil }
    return documentContentController.geometry(in: documentContentController.view)
  }

  public init(document: EVDocument, editorSurface: any EVEditorSurface, placement: EVDocumentWindowPlacement? = nil,
              diffPlacement: EVDocumentWindowPlacement? = nil, configuration: EVConfigurationStore? = nil) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.placement = placement ?? .shared
    // An injected placement also stands in for diff windows unless one is given.
    self.diffPlacement = diffPlacement ?? placement ?? .sharedDiff
    let firstPane = EVDocumentContentViewController(editorSurface: editorSurface, configuration: configuration)
    firstPane.document = document
    if let owner = Self.instances.compactMap(\.value).first(where: {
      !$0.isClosed && $0.paneContainer.panes.contains(where: { $0.document === document })
    }), let existing = owner.paneContainer.panes.first(where: { $0.document === document }) {
      firstPane.argumentList = existing.argumentList
      firstPane.argumentIndex = existing.argumentIndex
      argumentDocumentOpener = owner.argumentDocumentOpener
    }
    paneContainer = EVPaneContainer(first: firstPane)

    let window = EVDocumentWindow(
      contentRect: NSRect(origin: .zero, size: Self.initialContentSize),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    let contentView = paneContainer.view
    contentView.frame = NSRect(origin: .zero, size: Self.initialContentSize)
    contentView.translatesAutoresizingMaskIntoConstraints = false
    window.contentView = contentView
    window.contentMinSize = Self.minimumContentSize
    window.setContentSize(Self.initialContentSize)
    // Automatic tabbing can replace a just-created window's requested
    // frame with the geometry of an unrelated existing tab group. Viem's
    // initial UI has document windows, not a tab model, so opt out here.
    window.tabbingMode = .disallowed
    // State restoration needs a restoration class and stable document
    // identity. Advertising restoration without either can resurrect a
    // newly created untitled window in a stale miniaturized state.
    window.isRestorable = false
    window.titleVisibility = .visible

    super.init(window: window)
    paneContainer.splitForStatusDrag = { [weak self] source, bottom in
      guard let self, let document = source.document else { throw EVDocumentHostError.staleRequest }
      try self.paneContainer.requireSplitRoom(in: source)
      let pane = self.makePane(document: document, from: source)
      try self.paneContainer.insert(pane, splitting: source, focusNewPane: false,
                                    initialStatusBarBottom: bottom)
      self.updateActiveDocumentChrome()
      return pane
    }
    paneContainer.closePaneRequest = { [weak self] pane in self?.closePane(pane) }
    firstPane.editorMenuCommandDidPerform = { [weak self, weak firstPane] command, surface in
      guard let self, let firstPane else { return }
      self.editorMenuCommandDidPerform(command, on: surface, in: firstPane)
    }
    paneContainer.panesDidChange = { [weak self] in self?.updateActiveDocumentChrome() }
    window.delegate = self
    Self.instances.removeAll { $0.value == nil }
    Self.instances.append(EVWeakDocumentWindow(self))
    // NSDocument.addWindowController(_:) is the sole owner of attaching
    // this controller to its document. Pre-setting `document` here makes
    // AppKit treat the subsequent add as a no-op, leaving the document with
    // no retained window controllers.
    // Placement is applied explicitly once for programmatically built windows.
    shouldCascadeWindows = false
    documentContentController.document = document
    (editorSurface as? any EVDocumentHostAttachable)?.documentHostEffectHandler = self
    updateActiveDocumentChrome()
  }

  public static func windowShowing(document: EVDocument) -> NSWindow? {
    instances.compactMap(\.value).first { controller in
      !controller.isClosed && controller.paneContainer.panes.contains { $0.document === document }
    }?.window
  }

  static func refreshStatusFilePaths(for document: EVDocument) {
    for controller in instances.compactMap(\.value) where !controller.isClosed {
      for pane in controller.paneContainer.panes where pane.document === document {
        pane.refreshStatusFilePath()
      }
    }
  }

  var paneCount: Int { paneContainer.panes.count }

  /// `CTRL-W` window effects. Pane order, focus, and heights live in the pane
  /// container; a request from a pane that is no longer in this window is
  /// ignored rather than applied to someone else's panes.
  public func isCurrentScriptSurface(_ surface: any EVEditorSurface) -> Bool {
    !isClosed && paneContainer.activePane.editorSurface === surface
  }

  public func performScript(windowRequests: [EVWindowRequest], from surface: any EVEditorSurface) throws {
    guard isCurrentScriptSurface(surface) else { throw EVDocumentHostError.staleRequest }
    for request in windowRequests {
      if request == .closeOthers { try closeOtherPanes(force: false) }
      else { paneContainer.perform(request) }
    }
    updateActiveDocumentChrome()
  }

  public func perform(windowRequests: [EVWindowRequest], from surface: any EVEditorSurface) {
    guard paneContainer.panes.contains(where: { $0.editorSurface === surface }) else { return }
    for request in windowRequests {
      if request == .closeOthers {
        do { try closeOtherPanes(force: false) }
        catch { surface.showDocumentMessage(error.localizedDescription) }
      } else { paneContainer.perform(request) }
    }
    updateActiveDocumentChrome()
  }

  /// Captured before an open panel or recovery prompt can run a nested event
  /// loop. Only that exact, still-pristine blank window may be repurposed.
  @MainActor
  struct UntitledReplacement {
    fileprivate weak var controller: EVDocumentWindowController?
    fileprivate weak var document: EVDocument?
    fileprivate weak var pane: EVDocumentContentViewController?
    fileprivate var state: EVDocumentPersistenceState

    @discardableResult
    func install(_ opened: EVDocument) -> Bool {
      guard let controller, let document, let pane else { return false }
      return controller.replacePristineUntitled(document, in: pane, expected: state, with: opened)
    }
  }

  static func isPristineUntitled(_ document: EVDocument) -> Bool {
    let state = document.editorBackend.persistenceState
    return document.fileURL == nil && !document.isDocumentEdited
      && !state.isDirty && !state.isRecovered && !document.wasRecovered
      && state.sourceByteCount == 0 && state.documentRevision == 0
  }

  func captureUntitledReplacement() -> UntitledReplacement? {
    guard !isClosed, !isPerformingDocumentHostEffect, !closingAfterReview,
      paneCount == 1, let document = activeDocument, Self.isPristineUntitled(document)
    else { return nil }
    return UntitledReplacement(controller: self, document: document,
      pane: documentContentController, state: document.editorBackend.persistenceState)
  }

  private func replacePristineUntitled(
    _ original: EVDocument, in pane: EVDocumentContentViewController,
    expected: EVDocumentPersistenceState, with opened: EVDocument
  ) -> Bool {
    guard !isClosed, !isPerformingDocumentHostEffect, !closingAfterReview,
      paneCount == 1, documentContentController === pane,
      activeDocument === original, Self.isPristineUntitled(original),
      original.editorBackend.persistenceState.documentID == expected.documentID,
      original.editorBackend.persistenceState.documentRevision == expected.documentRevision
    else { return false }
    replaceActivePane(with: opened)
    closeIfUnrepresented(original)
    showWindow(nil)
    return true
  }

  func updateActiveDocumentChrome() {
    guard !isClosed, let document = activeDocument else { return }
    formattingToolbar.synchronize(surface: editorSurface)
    diffMode?.activeDocumentDidChange()
    window?.title =
      document.displayName
      + (paneContainer.panes.count > 1 ? " · \(paneContainer.panes.count) panes" : "")
    window?.representedURL = document.fileURL
    window?.isDocumentEdited = document.editorBackend.persistenceState.isDirty
  }

  private func rebindWindowDocument() {
    let owner = self.document as? EVDocument
    guard !paneContainer.panes.contains(where: { $0.document === owner }), let next = activeDocument
    else { return }
    owner?.removeWindowController(self)
    next.addWindowController(self)
  }

  public func windowDidBecomeKey(_ notification: Notification) {
    do { try editorSurface.fireAutocmd("FocusGained", matched: "") }
    catch { editorSurface.showDocumentMessage(error.localizedDescription) }
    updateActiveDocumentChrome()
    var seen = Set<ObjectIdentifier>()
    for pane in paneContainer.panes {
      guard let document = pane.document, seen.insert(ObjectIdentifier(document)).inserted else { continue }
      document.requestExternalFileReview()
    }
  }

  public func windowDidResignKey(_ notification: Notification) {
    do { try editorSurface.fireAutocmd("FocusLost", matched: "") }
    catch { editorSurface.showDocumentMessage(error.localizedDescription) }
  }

  func fireDocumentAutocmd(_ document: EVDocument, event: String, matched: String) throws -> Bool {
    guard let pane = paneContainer.panes.first(where: { $0.document === document }) else { return false }
    try pane.editorSurface.fireAutocmd(event, matched: matched)
    return true
  }

  public func windowShouldClose(_ sender: NSWindow) -> Bool {
    if closingAfterReview { return true }
    // Attached document windows enter through NSDocument's earlier preflight.
    // Retain this route for a window whose controller has not been attached.
    reviewDocumentsForClose { [weak self] approved in
      if approved { DispatchQueue.main.async { self?.close() } }
    }
    return false
  }

  func reviewDocumentsForClose(completion: @escaping (Bool) -> Void) {
    guard !isClosed, closeReviewCompletion == nil else { completion(false); return }
    closeReviewCompletion = completion
    var seen = Set<ObjectIdentifier>()
    closeQueue = paneContainer.panes.compactMap(\.document).filter { doc in
      guard seen.insert(ObjectIdentifier(doc)).inserted else { return false }
      let outside = Self.instances.compactMap(\.value).filter { $0 !== self && !$0.isClosed }
        .contains { $0.paneContainer.panes.contains { $0.document === doc } }
      return !outside && (doc.editorBackend.persistenceState.isDirty || doc.isDocumentEdited)
    }
    reviewNextClose()
  }

  private func reviewNextClose() {
    guard let document = closeQueue.first else {
      closingAfterReview = true
      let completion = closeReviewCompletion
      closeReviewCompletion = nil
      completion?(true)
      return
    }
    document.canClose(
      withDelegate: self, shouldClose: #selector(reviewedDocument(_:shouldClose:contextInfo:)),
      contextInfo: nil)
  }

  @objc private func reviewedDocument(
    _ document: NSDocument, shouldClose: Bool, contextInfo: UnsafeMutableRawPointer?
  ) {
    guard closeQueue.first === document else { return }
    guard shouldClose else {
      closeQueue.removeAll()
      closingAfterReview = false
      let completion = closeReviewCompletion
      closeReviewCompletion = nil
      completion?(false)
      return
    }
    if !closeQueue.isEmpty { closeQueue.removeFirst() }
    reviewNextClose()
  }

  public func windowWillClose(_ notification: Notification) {
    guard !isClosed else { return }
    // Record before leaving diff mode, so a diff window's frame stays a diff frame.
    if hasPresentedInitialWindow, let window { activePlacement.record(window: window, gutterWidth: placementGutterWidth) }
    diffMode?.disable()
    isClosed = true
    let documents = paneContainer.panes.compactMap(\.document)
    (document as? EVDocument)?.removeWindowController(self)
    var seen = Set<ObjectIdentifier>()
    for document in documents where seen.insert(ObjectIdentifier(document)).inserted {
      closeIfUnrepresented(document)
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  public override func windowDidLoad() {
    super.windowDidLoad()
    window?.makeFirstResponder(editorSurface.viewController.view)
  }

  public override func showWindow(_ sender: Any?) {
    EVStartupPerformance.mark("window.show.begin")
    defer { EVStartupPerformance.mark("window.show.end") }
    guard let window else {
      super.showWindow(sender)
      return
    }

    let isInitialPresentation = !hasPresentedInitialWindow
    if isInitialPresentation {
      prepareInitialFrame(of: window)
    }
    super.showWindow(sender)
    if window.isMiniaturized {
      window.deminiaturize(sender)
    }
    if isInitialPresentation {
      // Apply once more after AppKit has ordered the programmatic window.
      // This defeats any pre-show tiling/restoration candidate without
      // fighting subsequent user resizing.
      prepareInitialFrame(of: window)
      hasPresentedInitialWindow = true
      activePlacement.record(window: window, gutterWidth: placementGutterWidth)
    }
    _ = synchronizeContentFrame(of: window)
    window.makeFirstResponder(editorSurface.viewController.view)
    window.makeKeyAndOrderFront(sender)
  }

  private var placementGutterWidth: CGFloat {
    paneContainer.placementGutterWidth
  }

  private func prepareInitialFrame(of window: NSWindow) {
    if initialWindowFrame == nil {
      initialWindowFrame = activePlacement.initialFrame(for: window, defaultContentSize: Self.initialContentSize)
    }
    if let initialWindowFrame { window.setFrame(activePlacement.fittedFrame(EVDocumentWindowPlacement.includingGutter(initialWindowFrame, width: placementGutterWidth)), display: false) }
    _ = synchronizeContentFrame(of: window)
  }

  public func windowDidMove(_ notification: Notification) { recordWindowPlacement() }
  public func windowDidResize(_ notification: Notification) { recordWindowPlacement() }

  private func recordWindowPlacement() {
    guard hasPresentedInitialWindow, !isClosed, let window else { return }
    activePlacement.record(window: window, gutterWidth: placementGutterWidth)
  }

  @discardableResult
  private func synchronizeContentFrame(of window: NSWindow) -> NSView? {
    guard let contentView = window.contentView else { return nil }
    let contentSize = window.contentLayoutRect.size
    if contentView.frame.size != contentSize || contentView.frame.origin != .zero {
      contentView.frame = NSRect(origin: .zero, size: contentSize)
    }
    paneContainer.layoutPanes()
    return contentView
  }
}

@MainActor
extension EVDocumentWindowController {
  /// A command-line directory presents the same chooser as `:e {directory}`.
  /// The selection replaces a pristine placeholder; otherwise it opens in a
  /// new window so the files opened from the same command line stay visible.
  public func chooseFile(inLaunchDirectory directory: URL) {
    guard !isClosed, let document = activeDocument else { return }
    let state = document.editorBackend.persistenceState
    let kind: EVDocumentHostRequest.Kind = Self.isPristineUntitled(document) ? .edit : .editNewWindow
    perform(documentHostRequests: [EVDocumentHostRequest(
      kind: kind, documentID: state.documentID, documentRevision: state.documentRevision,
      path: directory.path)]) { [weak self] result in
      guard case let .failure(error) = result, (error as? CocoaError)?.code != .userCancelled else { return }
      self?.editorSurface.showDocumentMessage(error.localizedDescription)
    }
  }

  /// Startup and forwarded command lines install the same argument list.
  /// Already represented files keep their windows and panes; only new files
  /// occupy this initially hidden window. Later arguments stay lazy unless
  /// requested by `-o`, including when the first file was already open.
  /// A freshly loaded first document can already own the initial surface;
  /// accept only that exact document and filename instead of a pristine blank.
  /// Report the window selected for the first argument so a terminal launch
  /// can keep it in front when app activation completes.
  public func openArgumentList(
    _ urls: [URL], splitCount: Int?, initialLine: UInt64?, vertical: Bool = false,
    preparedFirstDocument: EVDocument? = nil,
    completion: @escaping @MainActor (Result<NSWindow?, Error>) -> Void
  ) {
    guard !isClosed, !isPerformingDocumentHostEffect,
      let original = activeDocument else {
      completion(.failure(EVDocumentHostError.operationAlreadyInProgress)); return
    }
    let hasPreparedFirstDocument = preparedFirstDocument === original && paneCount == 1
      && urls.first.map { first in
        original.fileURL.map { EVDocumentIdentity.sameFile($0, first) } ?? false
      } == true
    guard preparedFirstDocument == nil ? Self.isPristineUntitled(original) : hasPreparedFirstDocument else {
      completion(.failure(EVDocumentHostError.operationAlreadyInProgress)); return
    }
    let pane = documentContentController
    let state = original.editorBackend.persistenceState
    let list = EVArgumentList(urls)
    let argumentLimit = splitCount.map { $0 == 0 ? max(1, urls.count) : max(1, $0) } ?? 1
    var installedNewPane = false
    var firstTarget: (EVDocumentWindowController, EVDocumentContentViewController)?
    isPerformingDocumentHostEffect = true

    func finish(_ result: Result<Void, Error>) {
      if !installedNewPane && firstTarget != nil {
        // Every requested file was already shown. Retire the unused blank
        // without ever presenting another document window.
        close()
      } else if !isClosed {
        showWindow(nil)
        paneContainer.focusPane(at: 0)
        updateActiveDocumentChrome()
      }
      var selectedWindow: NSWindow?
      if let (controller, target) = firstTarget, !controller.isClosed,
        let index = controller.paneContainer.panes.firstIndex(where: { $0 === target }) {
        controller.paneContainer.focusPane(at: index)
        controller.updateActiveDocumentChrome()
        controller.showWindow(nil)
        selectedWindow = controller.window
      }
      finishDocumentHostEffect(result.map { _ in selectedWindow }, completion: completion)
    }

    // The native loader can complete synchronously. Drain those completions
    // iteratively so a long argv containing repeated files cannot exhaust the
    // stack; an asynchronous recovery result resumes the same drain later.
    var nextArgument: Int?
    var isOpeningArgument = false
    func openNext(_ index: Int) {
      nextArgument = index
      guard !isOpeningArgument else { return }
      isOpeningArgument = true
      defer { isOpeningArgument = false }
      while let next = nextArgument {
        nextArgument = nil
        openArgument(next)
      }
    }

    func openArgument(_ index: Int) {
      guard !isClosed else {
        finish(.failure(EVDocumentHostError.staleRequest)); return
      }
      guard index < argumentLimit else { finish(.success(())); return }
      // Reusing another window or an earlier argument consumes no space here.
      // Once this window is full, leave the next new target lazy without even
      // reading it, while allowing already represented arguments to be reused.
      if installedNewPane && paneCount >= launchPaneCapacity(vertical: vertical) {
        guard list.urls.indices.contains(index),
          let existing = EVDocumentIdentity.existingDocument(at: list.urls[index]),
          Self.windowShowing(document: existing) != nil else {
          finish(.success(())); return
        }
      }

      func installed(_ opened: EVDocument?, _ error: Error?) {
        guard let opened else {
          finish(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return
        }
        guard !isClosed else {
          closeIfUnrepresented(opened)
          finish(.failure(EVDocumentHostError.staleRequest)); return
        }
        let targetController: EVDocumentWindowController
        let targetPane: EVDocumentContentViewController
        if opened !== original || installedNewPane,
          let controller = Self.windowShowing(document: opened)?.windowController
          as? EVDocumentWindowController,
          let existingPane = controller.paneContainer.panes.first(where: { $0.document === opened }) {
          targetController = controller
          targetPane = existingPane
        } else {
          guard installedNewPane || (canReplace(pane, document: original, expected: state)
            && (hasPreparedFirstDocument || Self.isPristineUntitled(original))) else {
            closeIfUnrepresented(opened)
            finish(.failure(EVDocumentHostError.staleRequest)); return
          }
          if !installedNewPane {
            if opened !== original {
              replacePane(pane, with: opened)
              closeIfUnrepresented(original)
              targetPane = documentContentController
            } else {
              targetPane = pane
            }
            installedNewPane = true
          } else {
            paneContainer.focusPane(at: paneCount - 1)
            paneContainer.perform(vertical ? .setWidth(columns: nil) : .setHeight(rows: nil))
            do { try addPane(document: opened, vertical: vertical, before: false) } catch {
              closeIfUnrepresented(opened)
              finish(.failure(error)); return
            }
            paneContainer.perform(.equalizeHeights)
            targetPane = documentContentController
          }
          targetController = self
          showWindow(nil)
        }
        // Repeated/aliased filenames keep the first argument position in the
        // one existing pane while the complete list remains navigable.
        if targetPane.argumentList !== list {
          targetPane.argumentList = list
          targetPane.argumentIndex = list.urls.indices.contains(index) ? index : list.urls.indices.last
        }
        targetController.argumentDocumentOpener = argumentDocumentOpener
        if index == 0 {
          firstTarget = (targetController, targetPane)
          if let initialLine { targetPane.editorSurface.goToLine(initialLine) }
        }
        openNext(index + 1)
      }

      if index == 0 && hasPreparedFirstDocument {
        installed(original, nil)
      } else if list.urls.indices.contains(index) {
        loadArgumentDocument(list.urls[index], completion: installed)
      } else if index == 0 {
        installed(original, nil)
      } else {
        do {
          let blank = EVDocument()
          try blank.read(from: Data(), ofType: EVDocument.plainTextType)
          blank.fileType = EVDocument.plainTextType
          NSDocumentController.shared.addDocument(blank)
          installed(blank, nil)
        } catch { installed(nil, error) }
      }
    }
    openNext(0)
  }

  private func launchPaneCapacity(vertical: Bool = false) -> Int {
    if vertical { return max(1, Int(((window?.contentLayoutRect.width ?? 920) + 5) / 105)) }
    let minimum = EVStatusBarView.preferredHeight
    return max(1, Int((window?.contentLayoutRect.height ?? 680) / minimum))
  }

  fileprivate func loadArgumentDocument(
    _ url: URL, completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    if let argumentDocumentOpener { argumentDocumentOpener(url, completion) }
    else { openPaneDocument(url, completion: completion) }
  }

  fileprivate func canReplace(
    _ pane: EVDocumentContentViewController, document: EVDocument, expected: EVDocumentPersistenceState
  ) -> Bool {
    let current = document.editorBackend.persistenceState
    return !isClosed && paneContainer.panes.contains(where: { $0 === pane }) && pane.document === document
      && current.documentID == expected.documentID && current.documentRevision == expected.documentRevision
  }

  fileprivate func navigateArgument(
    _ document: EVDocument, request: EVDocumentHostRequest,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let pane = documentContentController
    guard pane.document === document, let navigation = request.argumentNavigation else {
      completion(.failure(EVDocumentHostError.staleRequest)); return
    }
    // In diff mode the argument commands step through the diff's files.
    let diffFiles = diffMode?.isEnabled == true ? diffMode?.files.map(\.url) : nil
    guard let urls = diffFiles ?? pane.argumentList?.urls, diffFiles?.isEmpty != true else {
      completion(.failure(EVArgumentListError.empty)); return
    }
    guard let resolve = EVArgumentListPolicy.resolve else {
      completion(.failure(EVDocumentHostError.unsupportedRequest)); return
    }
    let expected = document.editorBackend.persistenceState
    let preferred = diffFiles == nil ? pane.argumentIndex : diffMode?.currentFileIndex
    let currentIndex = diffFiles == nil ? pane.argumentList?.index(of: document.fileURL, preferring: preferred) : preferred
    let resolved = resolve(urls.count, currentIndex, preferred, navigation)

    func openTarget() {
      do {
        guard canReplace(pane, document: document, expected: expected) else { throw EVDocumentHostError.staleRequest }
        let index = try resolved.get()
        guard urls.indices.contains(index) else { throw EVArgumentListError.invalidIndex }
        let url = urls[index]
        let sameDocument = document.fileURL.map { EVDocumentIdentity.sameFile($0, url) } ?? false
        guard sameDocument || request.force || !document.editorBackend.persistenceState.isDirty
          || hasOtherView(of: document, excluding: pane) else { throw EVDocumentHostError.documentModified }
        if sameDocument {
          if diffFiles == nil { pane.argumentIndex = index }
          if let line = navigation.line { pane.editorSurface.goToLine(line == 0 ? UInt64.max : line) }
          completion(.success(nil)); return
        }
        loadArgumentDocument(url) { opened, error in
          guard let opened else { completion(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return }
          guard self.canReplace(pane, document: document, expected: expected) else {
            self.closeIfUnrepresented(opened)
            completion(.failure(EVDocumentHostError.staleRequest)); return
          }
          guard request.force || !document.editorBackend.persistenceState.isDirty
            || self.hasOtherView(of: document, excluding: pane) else {
            self.closeIfUnrepresented(opened)
            completion(.failure(EVDocumentHostError.documentModified)); return
          }
          self.replacePane(pane, with: opened)
          if diffFiles == nil { self.documentContentController.argumentIndex = index }
          self.closeIfUnrepresented(document)
          if let line = navigation.line { self.editorSurface.goToLine(line == 0 ? UInt64.max : line) }
          completion(.success(nil))
        }
      } catch { completion(.failure(error)) }
    }

    if navigation.writeFirst {
      let saved: @MainActor (Result<String?, Error>) -> Void = { result in
        switch result {
        case .success: openTarget()
        case .failure: completion(result)
        }
      }
      if request.path != nil {
        writeAlternate(document, request: request, closeAfter: false, completion: saved)
      } else {
        save(document, request: request, destination: document.fileURL,
          operation: document.fileURL == nil ? .saveAsOperation : .saveOperation, completion: saved)
      }
    } else { openTarget() }
  }

  fileprivate func finishDocumentHostEffect<Success>(
    _ result: Result<Success, Error>,
    completion: @escaping @MainActor (Result<Success, Error>) -> Void
  ) {
    isPerformingDocumentHostEffect = false
    completion(result)
  }

  fileprivate func performDocumentHostRequests(
    _ requests: ArraySlice<EVDocumentHostRequest>,
    messages: [String],
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard let request = requests.first else {
      finishDocumentHostEffect(
        .success(messages.isEmpty ? nil : messages.joined(separator: "\n")),
        completion: completion
      )
      return
    }
    performDocumentHostRequest(request) { [weak self] result in
      guard let self else {
        completion(.failure(EVDocumentHostError.unsupportedRequest))
        return
      }
      switch result {
      case .success(let message):
        var nextMessages = messages
        if let message, !message.isEmpty { nextMessages.append(message) }
        self.performDocumentHostRequests(
          requests.dropFirst(),
          messages: nextMessages,
          completion: completion
        )
      case .failure(let error):
        self.finishDocumentHostEffect(.failure(error), completion: completion)
      }
    }
  }

  private func applyScriptApplicationOption(_ name: String, value: String) throws {
    switch name {
    case "guifont", "linespace": try configuration.applyScriptCodeOption(name, value: value)
    case "background":
      let desired = value == "light" ? "Paper" : "Midnight"
      guard let theme = configuration.availableThemes.first(where: { $0.name.caseInsensitiveCompare(desired) == .orderedSame }) else {
        throw EVConfigurationStore.invalid("E185: Cannot find color scheme '\(desired)'")
      }
      try configuration.selectTheme(named: theme.name, fileName: theme.fileName)
    case "lines", "columns":
      guard let cells = Int(value), (name == "lines" ? 2...10000 : 12...10000).contains(cells), let window else {
        throw EVConfigurationStore.invalid("Invalid window size")
      }
      var size = window.contentLayoutRect.size
      let surface = editorSurface
      if name == "lines" { size.height = CGFloat(cells) * (surface.defaultLineHeight ?? 20) + documentContentController.statusBarHeight }
      else { size.width = CGFloat(cells) * (surface.defaultColumnWidth ?? 8) + surface.lineNumberGutterWidth }
      size.width = max(size.width, Self.minimumContentSize.width)
      size.height = max(size.height, Self.minimumContentSize.height)
      if let screen = window.screen { size.width = min(size.width, screen.visibleFrame.width); size.height = min(size.height, screen.visibleFrame.height) }
      window.setContentSize(size)
    case "guioptions":
      // macOS owns its application menu; only the cross-platform selection flag applies here.
      try editorSurface.applyScriptAutoselect(value.contains("a"))
    default:
      try configuration.scriptOptions.apply(name, value: value)
      for controller in Self.instances.compactMap(\.value) where controller.configuration === configuration && !controller.isClosed {
        for pane in controller.paneContainer.panes {
          if let visible = configuration.scriptOptions.showStatusBar { pane.setStatusBarVisibility(visible) }
          try pane.editorSurface.applyScriptViewOptions()
        }
      }
    }
  }

  fileprivate func performDocumentHostRequest(
    _ request: EVDocumentHostRequest,
    in targetPane: EVDocumentContentViewController? = nil,
    fileChosenInPanel: Bool = false,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    // Application settings requested by a script apply to every window.
    switch request.kind {
    case .theme:
      do {
        let name = request.path ?? ""
        let available = configuration.availableThemes
        if name.caseInsensitiveCompare("Default") == .orderedSame {
          try configuration.selectTheme(named: nil)
        } else if let theme = available.first(where: { $0.name.caseInsensitiveCompare(name) == .orderedSame }) {
          try configuration.selectTheme(named: theme.name, fileName: theme.fileName)
        } else {
          throw EVDocumentHostError.invalidPath(name)
        }
        completion(.success(nil))
      } catch { completion(.failure(error)) }
      return
    case .applicationOption:
      do {
        guard let text = request.path, let separator = text.firstIndex(of: "=") else { throw EVDocumentHostError.unsupportedRequest }
        let name = String(text[..<separator]), value = String(text[text.index(after: separator)...])
        try applyScriptApplicationOption(name, value: value)
        completion(.success(nil))
      } catch { completion(.failure(error)) }
      return
    default:
      break
    }
    guard
      let document = paneContainer.panes.compactMap(\.document).first(where: {
        $0.editorBackend.persistenceState.documentID == request.documentID
      })
    else {
      completion(.failure(EVDocumentHostError.unsupportedRequest))
      return
    }
    let persistence = document.editorBackend.persistenceState
    guard persistence.documentID == request.documentID,
      persistence.documentRevision == request.documentRevision
    else {
      completion(.failure(EVDocumentHostError.staleRequest))
      return
    }

    let openingPane = targetPane ?? documentContentController
    if [.edit, .editNewWindow, .split].contains(request.kind),
      let path = request.path, let url = resolvedFileURL(path, relativeTo: nil)
    {
      var isDirectory: ObjCBool = false
      let exists = FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory)
      if fileChosenInPanel && (!exists || isDirectory.boolValue) {
        completion(.failure(EVDocumentHostError.invalidPath(path)))
        return
      }
      if isDirectory.boolValue {
        chooseFile(in: url, document: document, pane: openingPane, request: request, completion: completion)
        return
      }
    }

    switch request.kind {
    case .read:
      do {
        guard let path = request.path ?? document.fileURL?.path,
              let url = resolvedFileURL(path, relativeTo: nil),
              let after = request.readAfterOffset,
              let pane = (documentContentController.document === document ? documentContentController : paneContainer.panes.first(where: { $0.document === document })) else {
          throw EVDocumentHostError.noDocumentURL
        }
        DispatchQueue.global(qos: .userInitiated).async {
          let loaded = Result { try Data(contentsOf: url) }
          DispatchQueue.main.async {
            do {
              let bytes = try loaded.get()
              guard self.paneContainer.panes.contains(where: { $0 === pane }), pane.document === document else {
                throw EVDocumentHostError.staleRequest
              }
              try pane.editorSurface.insertFileContents(bytes, after: after, source: request.readAfterIsSource, expected: persistence)
              completion(.success("\(url.path) read"))
            } catch { completion(.failure(error)) }
          }
        }
      } catch { completion(.failure(error)) }
    case .file:
      do {
        if let path = request.path {
          guard let url = resolvedFileURL(path, relativeTo: nil) else { throw EVDocumentHostError.invalidPath(path) }
          if let existing = EVDocumentIdentity.existingDocument(at: url), existing !== document {
            throw EVDocumentHostError.destinationExists(path)
          }
          if document.fileURL.map({ EVDocumentIdentity.sameFile($0, url) }) != true {
            document.fileURL = url
            // Renaming the buffer never acknowledges the new destination's
            // contents. An existing target still needs overwrite review.
            document.recordMissingFileBaseline(at: url)
            document.configureRecovery(for: url)
            updateActiveDocumentChrome()
          }
        }
        let name = document.fileURL?.path ?? "[No Name]"
        let flags = (persistence.isDirty ? " [Modified]" : "") + (persistence.isReadOnly ? " [Read only]" : "")
        completion(.success("\(name)\(flags)"))
      } catch { completion(.failure(error)) }
    case .only:
      do { try closeOtherPanes(force: request.force); completion(.success(nil)) }
      catch { completion(.failure(error)) }
    case .navigateArgument:
      navigateArgument(document, request: request, completion: completion)
    case .checkTime:
      document.checkForExternalChangesAndReview(completion: completion)
    case .theme, .applicationOption:
      // Handled before document validation.
      completion(.success(nil))
    case .split:
      split(document, in: openingPane, path: request.path, initialHeightRows: request.initialHeightRows, vertical: request.verticalSplit, completion: completion)
    case .newPane:
      do {
        try paneContainer.requireSplitRoom(vertical: request.verticalSplit)
        let next = EVDocument()
        try next.read(from: Data(), ofType: EVDocument.plainTextType)
        next.fileType = EVDocument.plainTextType
        NSDocumentController.shared.addDocument(next)
        try addPane(document: next, initialHeightRows: request.initialHeightRows, vertical: request.verticalSplit)
        completion(.success(nil))
      } catch {
        completion(.failure(error))
      }
    case .write:
      if request.path != nil || request.sourceByteRange != nil {
        writeAlternate(document, request: request, closeAfter: false, completion: completion)
        return
      }
      guard request.path == nil, request.sourceByteRange == nil else {
        completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
        return
      }
      save(
        document,
        request: request,
        destination: document.fileURL,
        operation: document.fileURL == nil ? .saveAsOperation : .saveOperation,
        completion: completion
      )

    case .saveAs:
      writeAlternate(document, request: request, closeAfter: false, adoptBinding: true, completion: completion)

    case .writeQuit:
      if request.path != nil || request.sourceByteRange != nil {
        writeAlternate(document, request: request, closeAfter: true, completion: completion)
        return
      }
      guard request.path == nil, request.sourceByteRange == nil else {
        completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
        return
      }
      save(
        document,
        request: request,
        destination: document.fileURL,
        operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
      ) { [weak self] result in
        guard case .success = result else {
          completion(result)
          return
        }
        guard !document.editorBackend.persistenceState.isDirty else {
          completion(.failure(EVDocumentHostError.documentModified))
          return
        }
        self?.closeCurrentDocumentOrWindow(document)
        completion(result)
      }

    case .xit:
      if request.path != nil {
        writeAlternate(document, request: request, closeAfter: true, completion: completion)
        return
      }
      guard request.path == nil else {
        completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
        return
      }
      guard persistence.isDirty else {
        closeCurrentDocumentOrWindow(document)
        completion(.success(nil))
        return
      }
      save(
        document,
        request: request,
        destination: document.fileURL,
        operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
      ) { [weak self] result in
        guard case .success = result else {
          completion(result)
          return
        }
        guard !document.editorBackend.persistenceState.isDirty else {
          completion(.failure(EVDocumentHostError.documentModified))
          return
        }
        self?.closeCurrentDocumentOrWindow(document)
        completion(result)
      }

    case .quit:
      guard
        request.force || !persistence.isDirty
          || hasOtherView(of: document, excluding: documentContentController)
      else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      closeCurrentDocumentOrWindow(document)
      completion(.success(nil))

    case .cquit:
      // Publish failure before closing any document can publish success.
      EVBlockingEditSessions.shared.abort(exitCode: request.exitStatus)
      let documents = NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }
      for controller in Self.instances.compactMap(\.value) where !controller.isClosed { controller.close() }
      for candidate in documents { candidate.close() }
      completion(.success(nil))
      terminateWithStatus(request.exitStatus)

    case .quitAll:
      var documents = NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }
      if !documents.contains(where: { $0 === document }) { documents.append(document) }
      guard request.force || documents.allSatisfy({ !$0.editorBackend.persistenceState.isDirty })
      else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      for candidate in documents { candidate.close() }
      commandDidCloseWindow()
      completion(.success(nil))

    case .writeAll:
      var documents = NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }
      if !documents.contains(where: { $0 === document }) { documents.append(document) }
      saveAll(documents[...], force: request.force, completion: completion)

    case .printWorkingDirectory:
      completion(.success(FileManager.default.currentDirectoryPath))

    case .changeDirectory:
      let path = request.path ?? NSHomeDirectory()
      guard let destination = resolvedFileURL(path, relativeTo: nil),
        FileManager.default.changeCurrentDirectoryPath(destination.path) else {
        completion(.failure(EVDocumentHostError.invalidPath(path)))
        return
      }
      for controller in Self.instances.compactMap(\.value) where !controller.isClosed {
        for pane in controller.paneContainer.panes { pane.refreshStatusFilePath() }
        if controller.diffMode?.isEnabled == true { controller.diffMode?.refreshToolbar() }
      }
      completion(.success(FileManager.default.currentDirectoryPath))

    case .editNewWindow:
      guard let path = request.path ?? document.fileURL?.path,
        let destination = resolvedFileURL(path, relativeTo: nil) else {
        completion(.failure(EVDocumentHostError.noDocumentURL))
        return
      }
      let sourcePane = openingPane
      openPaneDocument(destination) { opened, error in
        guard let opened else { completion(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return }
        guard self.canReplace(sourcePane, document: document, expected: persistence) else {
          self.closeIfUnrepresented(opened)
          completion(.failure(EVDocumentHostError.staleRequest))
          return
        }
        let controller = EVDocumentWindowController(document: opened, editorSurface: opened.editorBackend.makeEditorSurface())
        controller.argumentDocumentOpener = self.argumentDocumentOpener
        controller.documentContentController.argumentList = sourcePane.argumentList
        let remembered = sourcePane.argumentList?.index(of: document.fileURL, preferring: sourcePane.argumentIndex)
          ?? sourcePane.argumentIndex
        controller.documentContentController.argumentIndex = sourcePane.argumentList?.index(of: opened.fileURL,
          preferring: remembered) ?? remembered
        opened.addWindowController(controller)
        controller.showWindow(nil)
        completion(.success(nil))
      }

    case .edit:
      let reloadsCurrent = request.path == nil || request.path.flatMap { resolvedFileURL($0, relativeTo: nil) }
        .map { url in document.fileURL.map { EVDocumentIdentity.sameFile($0, url) } ?? false } == true
      guard request.force || !persistence.isDirty
        || (!reloadsCurrent && hasOtherView(of: document, excluding: openingPane)) else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      edit(document, in: openingPane, path: request.path, force: request.force, completion: completion)

    case .jumpFileMark(let name, let linewise):
      do {
        let target = try openingPane.editorSurface.resolveFileMark(name)
        for controller in Self.instances.compactMap(\.value) where !controller.isClosed {
          if let pane = controller.paneContainer.panes.first(where: { $0.document?.editorBackend.persistenceState.documentID == target.documentID }) {
            if let index = controller.paneContainer.panes.firstIndex(where: { $0 === pane }) { controller.paneContainer.focusPane(at: index) }
            controller.window?.makeKeyAndOrderFront(nil)
            try pane.editorSurface.jumpToFileMark(target, linewise: linewise)
            completion(.success(nil)); return
          }
        }
        guard !target.path.isEmpty else { throw EVDocumentHostError.noDocumentURL }
        guard !persistence.isDirty || hasOtherView(of: document, excluding: openingPane) else { throw EVDocumentHostError.documentModified }
        edit(document, in: openingPane, path: target.path, force: false) { [weak self] result in
          do {
            _ = try result.get()
            guard let self else { throw EVDocumentHostError.staleRequest }
            try self.documentContentController.editorSurface.jumpToFileMark(target, linewise: linewise)
            completion(.success(nil))
          } catch { completion(.failure(error)) }
        }
      } catch { completion(.failure(error)) }

    case .editAlternate:
      guard let alternate = openingPane.alternateURL else {
        completion(.failure(EVDocumentHostError.noAlternateFile))
        return
      }
      guard request.force || !persistence.isDirty || hasOtherView(of: document, excluding: openingPane) else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      edit(document, in: openingPane, path: alternate.path, force: request.force, completion: completion)

    case .new:
      guard
        request.force || !persistence.isDirty
          || hasOtherView(of: document, excluding: documentContentController)
      else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      do {
        let newDocument = try NSDocumentController.shared.makeUntitledDocument(
          ofType: EVDocument.plainTextType
        )
        NSDocumentController.shared.addDocument(newDocument)
        if let next = newDocument as? EVDocument {
          replaceActivePane(with: next)
          closeIfUnrepresented(document)
          completion(.success(nil))
        } else {
          completion(.failure(EVDocumentHostError.unsupportedRequest))
        }
      } catch {
        completion(.failure(error))
      }
    }
  }

  private func chooseFile(
    in directory: URL, document: EVDocument, pane: EVDocumentContentViewController,
    request: EVDocumentHostRequest,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let expected = document.editorBackend.persistenceState
    let fileURL = document.fileURL
    guard pane.document === document, let window, !isClosed else {
      completion(.failure(EVDocumentHostError.staleRequest))
      return
    }
    let panel = NSOpenPanel()
    panel.directoryURL = directory
    panel.allowsMultipleSelection = false
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsOtherFileTypes = true
    let selected: @MainActor (URL?) -> Void = { [weak self] url in
      guard let url else {
        completion(.failure(CocoaError(.userCancelled)))
        return
      }
      guard let self, self.canReplace(pane, document: document, expected: expected),
        document.fileURL == fileURL,
        document.editorBackend.persistenceState.isDirty == expected.isDirty
      else {
        completion(.failure(EVDocumentHostError.staleRequest))
        return
      }
      // Resolve the panel selection through the original command, retaining
      // its replacement/window/split destination, force flag, and split height.
      let selectedRequest = EVDocumentHostRequest(
        kind: request.kind, documentID: request.documentID, documentRevision: request.documentRevision,
        force: request.force, path: url.path, initialHeightRows: request.initialHeightRows,
        verticalSplit: request.verticalSplit)
      self.performDocumentHostRequest(selectedRequest, in: pane, fileChosenInPanel: true, completion: completion)
    }
    if let openFilePanelPresenter { openFilePanelPresenter(panel, window, selected) }
    else {
      panel.beginSheetModal(for: window) { response in
        selected(response == .OK ? panel.url : nil)
      }
    }
  }

  private func closeOtherPanes(force: Bool) throws {
    let active = paneContainer.activePane
    let removing = paneContainer.panes.filter { $0 !== active }
    var seen = Set<ObjectIdentifier>()
    let documents = removing.compactMap(\.document).filter { seen.insert(ObjectIdentifier($0)).inserted }
    for document in documents where document !== active.document {
      let survives = Self.instances.compactMap(\.value).contains { controller in
        !controller.isClosed && controller !== self
          && controller.paneContainer.panes.contains(where: { $0.document === document })
      }
      if !force && !survives && document.editorBackend.persistenceState.isDirty {
        throw EVDocumentHostError.documentModified
      }
    }
    for pane in removing { paneContainer.remove(pane) }
    rebindWindowDocument()
    for document in documents { closeIfUnrepresented(document) }
    updateActiveDocumentChrome()
  }


  fileprivate func writeAlternate(
    _ document: EVDocument, request: EVDocumentHostRequest, closeAfter: Bool, adoptBinding: Bool = false,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard !(document.isReadOnly || document.editorBackend.persistenceState.isReadOnly) || request.force else { completion(.failure(EVRecoveryError.readOnly)); return }
    guard let path = request.path ?? document.fileURL?.path, let destination = resolvedFileURL(path, relativeTo: nil) else {
      completion(.failure(EVDocumentHostError.invalidPath(request.path ?? ""))); return
    }
    do {
      try document.validateDestinationAlias(at: destination)
      let snapshot: EVDocumentSaveSnapshot
      if let range = request.sourceByteRange {
        snapshot = try document.editorBackend.nativeSaveSnapshot(typeName: document.fileType ?? EVDocument.plainTextType, sourceByteRange: range)
      } else { snapshot = try document.editorBackend.nativeSaveSnapshot(typeName: document.fileType ?? EVDocument.plainTextType) }
      guard snapshot.documentID == request.documentID, snapshot.documentRevision == request.documentRevision else {
        throw EVDocumentHostError.staleRequest
      }
      let writesCurrent = document.fileURL.map { EVDocumentIdentity.sameFile($0, destination) } ?? false
      if writesCurrent, !snapshot.isCompleteSource, !request.force { throw EVDocumentHostError.partialWriteRequiresForce }
      let target = EVDocumentIdentity.canonicalURL(destination)
      let authorization = try document.authorizeExternalWrite(to: target)
      document.beginExternalFileWrite()
      writeAuthorizedSnapshot(snapshot.data, document: document, to: target,
        force: request.force || writesCurrent || authorization.overwriteApproved,
        expected: authorization.fingerprint) { [weak self] result in
          defer { document.endExternalFileWrite() }
          do {
            try result.get()
            if writesCurrent || adoptBinding { document.recordRecentDocument(target) }
            if writesCurrent || adoptBinding { document.recordFileBaseline(snapshot.data, at: target) }
            if adoptBinding {
              document.fileURL = target
              document.configureRecovery(for: target)
            }
            if (writesCurrent && snapshot.isCompleteSource) || adoptBinding { try document.editorBackend.acknowledgeNativeSave(snapshot) }
            if closeAfter {
              let current = document.editorBackend.persistenceState
              guard current.documentID == snapshot.documentID, current.documentRevision == snapshot.documentRevision else {
                throw EVDocumentHostError.staleRequest
              }
              self?.closeCurrentDocumentOrWindow(document)
            }
            completion(.success("\(destination.path) written"))
          } catch { completion(.failure(error)) }
      }
    } catch { completion(.failure(error)) }
  }

  /// If the destination changes while a background writer prepares its temporary
  /// file, return to the document actor for a choice and retry the same snapshot.
  private func writeAuthorizedSnapshot(
    _ data: Data, document: EVDocument, to target: URL, force: Bool,
    expected: EVFileFingerprint, completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let result = Result { try EVExFileWriter.write(data, to: target, force: force, expected: expected) }
      DispatchQueue.main.async {
        if case .failure(let error) = result, error is EVExternalFileError {
          do {
            let accepted = try document.authorizeExternalWrite(to: target, since: expected)
            guard let self else { completion(.failure(CocoaError(.userCancelled))); return }
            self.writeAuthorizedSnapshot(data, document: document, to: target,
              force: force || accepted.overwriteApproved, expected: accepted.fingerprint, completion: completion)
          } catch { completion(.failure(error)) }
        } else { completion(result) }
      }
    }
  }

  fileprivate func save(
    _ document: EVDocument,
    request: EVDocumentHostRequest,
    destination: URL?,
    operation: NSDocument.SaveOperationType,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    if let destination {
      let typeName = documentType(for: destination, fallback: document.fileType)
      document.saveHostRevision(
        documentID: request.documentID,
        documentRevision: request.documentRevision,
        force: request.force,
        to: destination,
        ofType: typeName,
        for: operation
      ) { error in
        if let error {
          completion(.failure(error))
        } else {
          completion(.success("\(destination.path) written"))
        }
      }
      return
    }

    let panel = NSSavePanel()
    panel.canCreateDirectories = true
    panel.nameFieldStringValue =
      document.displayName == "Untitled"
      ? "Untitled.txt"
      : document.displayName
    guard document.prepareSavePanel(panel), let sheetWindow = document.windowForSheet else {
      completion(.failure(EVDocumentHostError.noDocumentURL))
      return
    }
    panel.beginSheetModal(for: sheetWindow) { [weak document] response in
      guard response == .OK, let document, let destination = panel.url else {
        completion(.failure(EVDocumentHostError.saveCancelledOrFailed))
        return
      }
      let typeName = self.documentType(for: destination, fallback: document.fileType)
      document.saveHostRevision(
        documentID: request.documentID,
        documentRevision: request.documentRevision,
        force: request.force,
        to: destination,
        ofType: typeName,
        for: .saveAsOperation
      ) { error in
        if let error {
          completion(.failure(error))
        } else {
          completion(.success("\(destination.path) written"))
        }
      }
    }
  }

  fileprivate func saveAll(
    _ documents: ArraySlice<EVDocument>,
    force: Bool,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard let document = documents.first else {
      completion(.success(nil))
      return
    }
    let state = document.editorBackend.persistenceState
    guard state.isDirty else {
      saveAll(documents.dropFirst(), force: force, completion: completion)
      return
    }
    let request = EVDocumentHostRequest(
      kind: .write,
      documentID: state.documentID,
      documentRevision: state.documentRevision,
      force: force
    )
    save(
      document,
      request: request,
      destination: document.fileURL,
      operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
    ) { result in
      switch result {
      case .success:
        self.saveAll(documents.dropFirst(), force: force, completion: completion)
      case .failure:
        completion(result)
      }
    }
  }

  fileprivate func split(
    _ source: EVDocument, in sourcePane: EVDocumentContentViewController,
    path: String?, initialHeightRows: Int? = nil, vertical: Bool = false,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let expected = source.editorBackend.persistenceState
    do { try paneContainer.requireSplitRoom(in: sourcePane, vertical: vertical) } catch { completion(.failure(error)); return }
    guard let path else {
      do { try addPane(document: source, initialHeightRows: initialHeightRows, splitting: sourcePane, vertical: vertical) }
      catch { completion(.failure(error)); return }
      completion(.success(nil))
      return
    }
    guard let url = resolvedFileURL(path, relativeTo: source.fileURL) else {
      completion(.failure(EVDocumentHostError.invalidPath(path)))
      return
    }
    openPaneDocument(url) { [weak self] opened, error in
      if let opened {
        guard let self else { completion(.failure(EVDocumentHostError.staleRequest)); return }
        do {
          guard self.canReplace(sourcePane, document: source, expected: expected)
          else { throw EVDocumentHostError.staleRequest }
          try self.addPane(document: opened, initialHeightRows: initialHeightRows, splitting: sourcePane, vertical: vertical)
          completion(.success(nil))
        } catch {
          self.closeIfUnrepresented(opened)
          completion(.failure(error))
        }
      } else {
        completion(.failure(error ?? EVDocumentHostError.unsupportedRequest))
      }
    }
  }

  fileprivate func openPaneDocument(
    _ url: URL, completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    Self.openPaneDocument(url, completion: completion)
  }

  fileprivate static func openPaneDocument(
    _ url: URL, completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    if !FileManager.default.fileExists(atPath: url.path),
      EVDocumentIdentity.existingDocument(at: url) == nil
    {
      do {
        let document = EVDocument()
        try document.openMissingFile(at: url)
        NSDocumentController.shared.addDocument(document)
        completion(document, nil)
      } catch { completion(nil, error) }
    } else {
      EVDocumentIdentity.open(url, display: false, completion: completion)
    }
  }

  fileprivate func edit(
    _ document: EVDocument, in pane: EVDocumentContentViewController,
    path: String?, force: Bool = false,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let url: URL
    if let path {
      guard let resolved = resolvedFileURL(path, relativeTo: document.fileURL) else {
        completion(.failure(EVDocumentHostError.invalidPath(path)))
        return
      }
      url = resolved
    } else if let current = document.fileURL {
      url = current
    } else {
      completion(.failure(EVDocumentHostError.noDocumentURL))
      return
    }
    if document.fileURL.map({ EVDocumentIdentity.sameFile($0, url) }) == true {
      do {
        try document.revert(
          toContentsOf: url, ofType: documentType(for: url, fallback: document.fileType))
        completion(.success(nil))
      } catch { completion(.failure(error)) }
      return
    }
    let state = document.editorBackend.persistenceState
    openPaneDocument(url) { [weak self] opened, error in
      if let opened, let self {
        guard self.canReplace(pane, document: document, expected: state) else {
          self.closeIfUnrepresented(opened)
          completion(.failure(EVDocumentHostError.staleRequest)); return
        }
        guard force || !document.editorBackend.persistenceState.isDirty
          || self.hasOtherView(of: document, excluding: pane) else {
          self.closeIfUnrepresented(opened)
          completion(.failure(EVDocumentHostError.documentModified)); return
        }
        self.replacePane(pane, with: opened)
        self.closeIfUnrepresented(document)
        completion(.success(nil))
      } else {
        completion(.failure(error ?? EVDocumentHostError.unsupportedRequest))
      }
    }
  }

  fileprivate func closeCurrentDocumentOrWindow(_ document: EVDocument) {
    if paneContainer.panes.count > 1 {
      let target = documentContentController
      paneContainer.remove(target)
      rebindWindowDocument()
      closeIfUnrepresented(document)
      updateActiveDocumentChrome()
    } else {
      close()
      commandDidCloseWindow()
    }
  }

  /// Close one view, as Close does for the window: an unsaved buffer without
  /// another view is reviewed first, and the last view closes the window.
  func closePane(_ pane: EVDocumentContentViewController) {
    guard !isClosed, closeReviewCompletion == nil, pendingPaneClose == nil,
      paneContainer.panes.contains(where: { $0 === pane }) else { return }
    guard paneContainer.panes.count > 1 else { window?.performClose(nil); return }
    guard let document = pane.document else { return }
    let finish = { [weak self, weak pane] in
      guard let self, let pane, !self.isClosed, self.paneContainer.panes.count > 1,
        self.paneContainer.panes.contains(where: { $0 === pane }) else { return }
      self.paneContainer.remove(pane)
      self.rebindWindowDocument()
      self.closeIfUnrepresented(document)
      self.updateActiveDocumentChrome()
    }
    guard (document.editorBackend.persistenceState.isDirty || document.isDocumentEdited)
      && !hasOtherView(of: document, excluding: pane) else { finish(); return }
    pendingPaneClose = finish
    document.canClose(
      withDelegate: self, shouldClose: #selector(reviewedPaneClose(_:shouldClose:contextInfo:)),
      contextInfo: nil)
  }

  @objc private func reviewedPaneClose(
    _ document: NSDocument, shouldClose: Bool, contextInfo: UnsafeMutableRawPointer?
  ) {
    let finish = pendingPaneClose
    pendingPaneClose = nil
    if shouldClose { finish?() }
  }

  fileprivate func hasOtherView(
    of document: EVDocument, excluding pane: EVDocumentContentViewController?
  ) -> Bool {
    Self.instances.compactMap(\.value).filter { !$0.isClosed }.contains { controller in
      controller.paneContainer.panes.contains { $0 !== pane && $0.document === document }
    }
  }

  fileprivate func closeIfUnrepresented(_ document: EVDocument) {
    guard !hasOtherView(of: document, excluding: nil), document.windowControllers.isEmpty else {
      return
    }
    document.close()
  }

  /// A diff window compares every pane it shows; its views are Code from
  /// their first frame, so a Markdown file never shows Preview first.
  var isDiffWindow: Bool { opensForDiff || diffMode?.isEnabled == true }

  /// Mark a window opened for comparison: its first pane and every later
  /// one show Code.
  func markOpensForDiff() {
    opensForDiff = true
    for pane in paneContainer.panes { pane.editorSurface.requestedDisplay = .diff }
  }

  fileprivate func makePane(document: EVDocument, from source: EVDocumentContentViewController? = nil) -> EVDocumentContentViewController {
    // A diff window's panes are Code; a split shows what its source shows.
    var display = EVViewDisplay.default
    if isDiffWindow {
      display = .diff
    } else if let source, source.document === document, source.editorSurface.currentDisplay != .diff {
      display = source.editorSurface.currentDisplay
    }
    let surface = document.editorBackend.makeEditorSurface(display: display)
    let pane = EVDocumentContentViewController(editorSurface: surface, configuration: configuration)
    pane.document = document
    let previous = source ?? documentContentController
    surface.inheritLineNumberOption(from: previous.editorSurface)
    // A split keeps its source pane's alternate file, as Vim's windows do.
    pane.alternateURL = previous.alternateURL
    pane.argumentList = previous.argumentList
    pane.argumentIndex = previous.argumentList?.index(of: previous.document?.fileURL,
      preferring: previous.argumentIndex) ?? previous.argumentIndex
    if let matched = pane.argumentList?.index(of: document.fileURL, preferring: pane.argumentIndex) {
      pane.argumentIndex = matched
    }
    (surface as? any EVDocumentHostAttachable)?.documentHostEffectHandler = self
    pane.editorMenuCommandDidPerform = { [weak self, weak pane] command, surface in
      guard let self, let pane else { return }
      self.editorMenuCommandDidPerform(command, on: surface, in: pane)
    }
    return pane
  }

  fileprivate func addPane(document: EVDocument, initialHeightRows: Int? = nil, splitting source: EVDocumentContentViewController? = nil, vertical: Bool = false, before: Bool = true) throws {
    try paneContainer.requireSplitRoom(in: source, vertical: vertical)
    try paneContainer.insert(makePane(document: document, from: source), splitting: source, vertical: vertical, before: before)
    if let rows = initialHeightRows {
      paneContainer.perform(vertical ? .setWidth(columns: rows) : .setHeight(rows: rows))
    }
    updateActiveDocumentChrome()
  }

  fileprivate func replaceActivePane(with document: EVDocument) {
    replacePane(documentContentController, with: document)
  }

  fileprivate func replacePane(_ old: EVDocumentContentViewController, with document: EVDocument) {
    EVStartupPerformance.mark("window.replacePane.begin")
    defer { EVStartupPerformance.mark("window.replacePane.end") }
    let next = makePane(document: document)
    // The file this pane showed becomes its alternate file (`CTRL-^`).
    next.alternateURL = old.document?.fileURL ?? old.alternateURL
    next.argumentList = old.argumentList
    let previousIndex = old.argumentList?.index(of: old.document?.fileURL, preferring: old.argumentIndex)
      ?? old.argumentIndex
    next.argumentIndex = old.argumentList?.index(of: document.fileURL, preferring: previousIndex) ?? previousIndex
    paneContainer.replace(old, with: next)
    rebindWindowDocument()
    updateActiveDocumentChrome()
  }

  fileprivate func resolvedFileURL(_ path: String, relativeTo currentURL: URL?) -> URL? {
    guard !path.isEmpty, !path.utf8.contains(0) else { return nil }
    let expanded = (path as NSString).expandingTildeInPath
    if expanded.hasPrefix("/") {
      return URL(fileURLWithPath: expanded).standardizedFileURL
    }
    let base = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
    return URL(fileURLWithPath: expanded, relativeTo: base).standardizedFileURL
  }

  fileprivate func documentType(for url: URL, fallback: String?) -> String {
    switch url.pathExtension.lowercased() {
    case "md", "markdown", "mdown": EVDocument.markdownType
    case "html", "htm", "xhtml": EVDocument.codeType
    case "txt", "text": EVDocument.plainTextType
    default: fallback ?? EVDocument.plainTextType
    }
  }
}

@MainActor
extension EVDocumentWindowController {
  public func openLinkedDocument(
    _ url: URL, fragment: String?, from surface: any EVEditorSurface,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    openLinkedDocument(url, fragment: fragment, from: surface, using: { url, finished in
      EVDocumentIdentity.open(url, display: false, completion: finished)
    }, completion: completion)
  }

  func openLinkedDocument(
    _ url: URL, fragment: String?, from surface: any EVEditorSurface,
    using open: @escaping @MainActor (URL, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    guard url.isFileURL else {
      completion(.failure(EVDocumentHostError.invalidPath(url.absoluteString))); return
    }
    guard !isClosed, paneContainer.panes.contains(where: { $0.editorSurface === surface }) else {
      completion(.failure(EVDocumentHostError.staleRequest)); return
    }
    if let target = existingLinkedDocumentView(at: url) {
      target.controller.presentLinkedDocumentView(target.pane, fragment: fragment, completion: completion)
      return
    }
    open(url) { [weak self, weak surface] opened, error in
      guard let opened else {
        completion(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return
      }
      guard let self, !self.isClosed, let surface,
        self.paneContainer.panes.contains(where: { $0.editorSurface === surface }) else {
        if opened.windowControllers.isEmpty && !Self.hasOpenViews(of: opened) { opened.close() }
        completion(.failure(EVDocumentHostError.staleRequest)); return
      }
      // Another open may have published this file during asynchronous loading.
      // Reuse its current buffer and view rather than displaying a duplicate.
      if let target = self.existingLinkedDocumentView(at: url) {
        if target.pane.document !== opened { self.closeIfUnrepresented(opened) }
        target.controller.presentLinkedDocumentView(target.pane, fragment: fragment, completion: completion)
        return
      }
      opened.addEditorWindowController()
      guard let window = opened.windowControllers.last as? EVDocumentWindowController else {
        completion(.failure(EVDocumentHostError.unsupportedRequest)); return
      }
      window.presentLinkedDocumentView(window.documentContentController, fragment: fragment, completion: completion)
    }
  }

  private func existingLinkedDocumentView(at url: URL)
    -> (controller: EVDocumentWindowController, pane: EVDocumentContentViewController)? {
    // Prefer an already active view, with this window first and remaining
    // windows in native front-to-back order. Hidden windows retain creation
    // order. If only inactive panes match, use that same stable window order.
    var controllers: [EVDocumentWindowController] = []
    var seen = Set<ObjectIdentifier>()
    for controller in [self] + NSApplication.shared.orderedWindows.compactMap({
      $0.windowController as? EVDocumentWindowController
    }) + Self.instances.compactMap(\.value) where !controller.isClosed {
      if seen.insert(ObjectIdentifier(controller)).inserted { controllers.append(controller) }
    }
    func matches(_ pane: EVDocumentContentViewController) -> Bool {
      pane.document?.fileURL.map { EVDocumentIdentity.sameFile($0, url) } ?? false
    }
    for controller in controllers {
      let active = controller.paneContainer.activePane
      if matches(active) { return (controller, active) }
    }
    for controller in controllers {
      if let pane = controller.paneContainer.panes.first(where: matches) { return (controller, pane) }
    }
    return nil
  }

  private func presentLinkedDocumentView(
    _ pane: EVDocumentContentViewController, fragment: String?,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    guard !isClosed, let index = paneContainer.panes.firstIndex(where: { $0 === pane }) else {
      completion(.failure(EVDocumentHostError.staleRequest)); return
    }
    paneContainer.focusPane(at: index)
    updateActiveDocumentChrome()
    showWindow(nil)
    do {
      if let fragment {
        guard let navigation = pane.editorSurface as? any EVLinkFragmentNavigating else {
          throw EVDocumentHostError.unsupportedRequest
        }
        try navigation.navigateToLinkFragment(fragment)
      }
      if let document = pane.document, let url = document.fileURL {
        document.recordRecentDocument(EVDocumentIdentity.canonicalURL(url))
      }
      completion(.success(()))
    } catch {
      pane.editorSurface.showDocumentMessage(error.localizedDescription)
      completion(.failure(error))
    }
  }

  public func openDroppedFiles(
    _ urls: [URL], in targetSurface: any EVEditorSurface,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    openDroppedFiles(urls, in: targetSurface, using: { url, finished in
      EVDocumentIdentity.open(url, display: false, completion: finished)
    }, completion: completion)
  }

  func openDroppedFiles(
    _ urls: [URL], in targetSurface: any EVEditorSurface,
    using open: @escaping @MainActor (URL, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void,
    completion: @escaping @MainActor (Result<Void, Error>) -> Void
  ) {
    guard !isClosed, !isPerformingDocumentHostEffect,
      let target = paneContainer.panes.first(where: { $0.editorSurface === targetSurface }),
      let original = target.document else {
      completion(.failure(EVDocumentHostError.operationAlreadyInProgress)); return
    }
    guard !urls.isEmpty else { completion(.success(())); return }
    let originalState = original.editorBackend.persistenceState
    let replaceFirst = !originalState.isDirty && !original.isDocumentEdited
    isPerformingDocumentHostEffect = true
    func finish(_ result: Result<Void, Error>) {
      isPerformingDocumentHostEffect = false
      completion(result)
    }
    func next(_ remaining: ArraySlice<URL>, first: Bool) {
      guard let url = remaining.first else { finish(.success(())); return }
      guard url.isFileURL else { finish(.failure(EVDocumentHostError.invalidPath(url.absoluteString))); return }
      open(url) { opened, error in
        guard let opened else { finish(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return }
        // Opening may have displayed an asynchronous recovery or permission
        // sheet. Preserve any edits or pane replacement made while it was open.
        if first && replaceFirst && !self.isClosed,
          self.paneContainer.panes.contains(where: { $0 === target }),
          target.document === original,
          original.editorBackend.persistenceState.documentID == originalState.documentID,
          original.editorBackend.persistenceState.documentRevision == originalState.documentRevision,
          !original.editorBackend.persistenceState.isDirty && !original.isDocumentEdited {
          if opened !== original {
            self.replacePane(target, with: opened)
            self.closeIfUnrepresented(original)
          }
          self.updateActiveDocumentChrome()
        } else {
          let controller = EVDocumentWindowController(document: opened,
            editorSurface: opened.editorBackend.makeEditorSurface())
          opened.addWindowController(controller)
          controller.showWindow(nil)
        }
        let following = remaining.dropFirst()
        if following.isEmpty { finish(.success(())) }
        else { DispatchQueue.main.async { next(following, first: false) } }
      }
    }
    next(urls[...], first: true)
  }

  public func perform(
    documentHostRequests: [EVDocumentHostRequest],
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard !isPerformingDocumentHostEffect else {
      completion(.failure(EVDocumentHostError.operationAlreadyInProgress))
      return
    }
    isPerformingDocumentHostEffect = true
    performDocumentHostRequests(
      documentHostRequests[...],
      messages: [],
      completion: completion
    )
  }
}

private final class EVWeakDocumentWindow {
  weak var value: EVDocumentWindowController?
  init(_ value: EVDocumentWindowController) { self.value = value }
}

struct EVDocumentWindowGeometry: Equatable {
  let content: NSRect
  let editor: NSRect
  let statusBar: NSRect
  let statusBarIsVisible: Bool
}

@MainActor
final class EVDocumentContentViewController: NSViewController,
  EVEditorCommandRouting,
  NSMenuItemValidation
{
  weak var document: EVDocument? {
    didSet { refreshStatusFilePath() }
  }
  var argumentList: EVArgumentList?
  var argumentIndex: Int?
  /// The file this pane showed before its current document (`CTRL-^`).
  var alternateURL: URL? { didSet { editorSurface.setAlternateFile(alternateURL?.path) } }

  let editorSurface: any EVEditorSurface
  private let configuration: EVConfigurationStore
  let statusBar = EVStatusBarView()
  /// Runs after an editor menu command on either side of the pane, so the
  /// window can follow settings that diff mode and Code remember.
  var editorMenuCommandDidPerform: ((EVMenuCommand, any EVEditorSurface) -> Void)?
  var statusBarHeightDidChange: (() -> Void)?
  private var showsStatusBar: Bool

  init(editorSurface: any EVEditorSurface, configuration: EVConfigurationStore? = nil) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.editorSurface = editorSurface
    showsStatusBar = configuration.scriptOptions.showStatusBar ?? configuration.showStatusBar
    super.init(nibName: nil, bundle: nil)

    editorSurface.statusBarStateDidChange = { [weak self] _ in self?.statusStateDidChange() }
    statusBar.preferredHeightDidChange = { [weak self] in
      self?.layoutContent()
      self?.statusBarHeightDidChange?()
    }
    statusBar.optionDidChange = { [weak self] option in
      guard let self else { return }
      self.statusSurface.perform(statusOption: option)
      self.statusBar.apply(self.statusSurface.statusBarState)
      self.view.window?.makeFirstResponder(self.statusSurface.viewController.view)
    }
    statusBar.changeDirectory = { [weak self] _ in
      guard let self, let document = self.document,
        let directory = document.fileURL?.deletingLastPathComponent(),
        let controller = self.viewIfLoaded?.window?.windowController as? EVDocumentWindowController
      else { return }
      let state = document.editorBackend.persistenceState
      controller.performDocumentHostRequest(EVDocumentHostRequest(kind: .changeDirectory,
        documentID: state.documentID, documentRevision: state.documentRevision, path: directory.path), in: self) { [weak self] result in
        switch result {
        case .success(let message): if let message { self?.editorSurface.showDocumentMessage(message) }
        case .failure(let error): self?.editorSurface.showDocumentMessage(error.localizedDescription)
        }
      }
    }
    // The command line is drawn by the status line, so pointer selection in it
    // is reported from there and routed back to this pane's surface.
    statusBar.commandLineDidSelect = { [weak self] offset, extending in
      guard let self else { return }
      self.statusSurface.selectCommandLine(atUTF8Offset: offset, extending: extending)
    }
    statusBar.commandOutputDidDismiss = { [weak self] in
      self?.statusSurface.dismissCommandOutput()
    }
    statusBar.commandOutputDidReceiveKey = { [weak self] event in
      self?.statusSurface.handleStatusMessageKey(event)
    }
  }

  /// The surface whose state the status bar shows: a diff base while it has
  /// a command line, output or interaction of its own, otherwise the editor.
  private var statusSurface: any EVEditorSurface {
    if let base = diffBaseSurface {
      let state = base.statusBarState
      if state.commandLine != nil || state.commandOutput != nil || state.requiresInteraction { return base }
    }
    return editorSurface
  }

  private func statusStateDidChange() {
    let surface = statusSurface
    let state = surface.statusBarState
    let outputHadFocus = statusBar.isCommandOutputFocused
    statusBar.apply(state)
    updateDiffBaseStatus()
    if outputHadFocus, state.commandOutput == nil || state.commandLine != nil {
      viewIfLoaded?.window?.makeFirstResponder(surface.viewController.view)
    }
    // The command line lives in the status line, so a hidden status line
    // still has to appear while one is active.
    let needed = showsStatusBar || state.commandLine != nil || state.commandOutput != nil || state.requiresInteraction
    if statusBar.isHidden == needed {
      statusBar.isHidden = !needed
      layoutContent()
      statusBarHeightDidChange?()
    }
    (viewIfLoaded?.window?.windowController as? EVDocumentWindowController)?
      .updateActiveDocumentChrome()
  }

  // MARK: Diff pair

  /// In diff mode the base sits left of the editor above one status bar that
  /// spans both, so pane splits and status drags move the pair together.
  private var diffSplit: EVDiffSplitView?
  private(set) var diffBaseSurface: (any EVEditorSurface)?
  private var diffBaseTitle = ""
  private var diffBaseFileURL: URL?

  func attachDiffBase(_ base: any EVEditorSurface, baseFraction: CGFloat,
                      baseFractionDidChange: @escaping (CGFloat) -> Void) {
    loadViewIfNeeded()
    guard diffBaseSurface !== base else { return }
    detachDiffBase()
    diffBaseSurface = base
    addChild(base.viewController)
    let split = EVDiffSplitView(base: base.viewController.view, document: editorSurface.viewController.view,
                                baseFraction: baseFraction)
    split.translatesAutoresizingMaskIntoConstraints = true
    split.autoresizingMask = []
    split.baseFractionDidChange = baseFractionDidChange
    split.didLayout = { [weak self] in self?.updateDiffBaseStatus() }
    view.addSubview(split, positioned: .below, relativeTo: statusBar)
    diffSplit = split
    base.statusBarStateDidChange = { [weak self] _ in self?.statusStateDidChange() }
    layoutContent()
    statusStateDidChange()
  }

  func setDiffBaseStatus(title: String, fileURL: URL?) {
    diffBaseTitle = title
    diffBaseFileURL = fileURL
    updateDiffBaseStatus()
  }

  func detachDiffBase() {
    guard let split = diffSplit, let base = diffBaseSurface else { return }
    let baseView = base.viewController.view
    let baseHadFocus = (view.window?.firstResponder as? NSView)?.isDescendant(of: baseView) == true
    base.statusBarStateDidChange = nil
    diffSplit = nil
    diffBaseSurface = nil
    let editorView = editorSurface.viewController.view
    view.addSubview(editorView, positioned: .below, relativeTo: statusBar)
    baseView.removeFromSuperview()
    base.viewController.removeFromParent()
    split.removeFromSuperview()
    statusBar.diffBase = nil
    layoutContent()
    statusStateDidChange()
    if baseHadFocus { view.window?.makeFirstResponder(editorView) }
  }

  private func updateDiffBaseStatus() {
    guard let split = diffSplit, let base = diffBaseSurface else {
      if statusBar.diffBase != nil { statusBar.diffBase = nil }
      return
    }
    statusBar.diffBase = EVStatusBarView.DiffBase(title: diffBaseTitle, fileURL: diffBaseFileURL,
      location: base.statusBarState.location, width: split.document.frame.minX)
  }

  /// Editor commands go to the side of a diff pair that holds the focus.
  var commandSurface: any EVEditorSurface {
    if let base = diffBaseSurface, let responder = view.window?.firstResponder as? NSView,
       responder.isDescendant(of: base.viewController.view) { return base }
    return editorSurface
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  override func loadView() {
    let root = NSView()
    root.clipsToBounds = true
    // Programmatic content views do not receive the nib loader's default
    // width/height autoresizing mask. Keep the controller root matched to
    // the window's content rect as the window is shown and resized.
    root.autoresizingMask = [.width, .height]

    addChild(editorSurface.viewController)
    let editorView = editorSurface.viewController.view
    editorView.translatesAutoresizingMaskIntoConstraints = true
    editorView.autoresizingMask = []
    statusBar.translatesAutoresizingMaskIntoConstraints = true
    statusBar.autoresizingMask = []
    root.addSubview(editorView)
    root.addSubview(statusBar)

    statusBar.apply(editorSurface.statusBarState)
    statusBar.isHidden = !showsStatusBar && editorSurface.statusBarState.commandLine == nil
      && editorSurface.statusBarState.commandOutput == nil
      && !editorSurface.statusBarState.requiresInteraction
    view = root
    layoutContent()
  }

  func geometry(in root: NSView) -> EVDocumentWindowGeometry {
    loadViewIfNeeded()
    layoutContent()
    let editorView = editorSurface.viewController.view
    return EVDocumentWindowGeometry(
      content: root.bounds,
      editor: editorView.convert(editorView.bounds, to: root),
      statusBar: statusBar.convert(statusBar.bounds, to: root),
      statusBarIsVisible: !statusBar.isHidden
    )
  }

  func refreshStatusFilePath() {
    statusBar.fileURL = document?.fileURL
  }

  var statusBarHeight: CGFloat { statusBar.isHidden ? 0 : statusBar.visiblePreferredHeight }

  func layoutContent() {
    loadViewIfNeeded()
    let bounds = view.bounds
    let statusHeight = statusBarHeight
    let editorView: NSView = diffSplit ?? editorSurface.viewController.view
    statusBar.frame = NSRect(
      x: bounds.minX,
      y: bounds.minY,
      width: bounds.width,
      height: statusHeight
    )
    // Explicit pane frames use DIPs throughout, including a status bar that
    // was hidden or belonged to a differently sized split.
    if statusBar.bounds.size != statusBar.frame.size { statusBar.setBoundsSize(statusBar.frame.size) }
    editorView.frame = NSRect(
      x: bounds.minX,
      y: bounds.minY + statusHeight,
      width: bounds.width,
      height: max(0, bounds.height - statusHeight)
    )
    // AppKit retains an old bounds dimension when a previously zero-sized
    // view is autoresized. Keep collapsed editor geometry exact as well.
    if editorView.bounds.size != editorView.frame.size { editorView.setBoundsSize(editorView.frame.size) }
    statusBar.layoutSubtreeIfNeeded()
    editorView.layoutSubtreeIfNeeded()
  }

  @objc(saveDocument:) func savePane(_ sender: Any?) { document?.save(sender) }
  @objc(saveDocumentAs:) func savePaneAs(_ sender: Any?) { document?.saveAs(sender) }

  @objc func performEditorMenuCommand(_ sender: Any?) {
    guard
      let menuItem = sender as? NSMenuItem,
      let command = EVMenuCommand(rawValue: menuItem.tag)
    else { return }

    if command == .newWindowForDocument {
      document?.showAdditionalWindow()
    } else {
      let surface = commandSurface
      surface.perform(menuCommand: command, sender: sender)
      editorMenuCommandDidPerform?(command, surface)
    }
  }

  func setStatusBarVisibility(_ visible: Bool) {
    showsStatusBar = visible
    statusBar.isHidden = !showsStatusBar && editorSurface.statusBarState.commandLine == nil
      && editorSurface.statusBarState.commandOutput == nil && !editorSurface.statusBarState.requiresInteraction
    layoutContent()
    statusBarHeightDidChange?()
  }

  @objc func toggleStatusBar(_ sender: Any?) {
    showsStatusBar.toggle()
    statusBar.isHidden = !showsStatusBar && editorSurface.statusBarState.commandLine == nil
      && editorSurface.statusBarState.commandOutput == nil
      && !editorSurface.statusBarState.requiresInteraction
    layoutContent()
    statusBarHeightDidChange?()
    try? configuration.setShowStatusBar(showsStatusBar)
  }

  func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
    if menuItem.action == #selector(toggleStatusBar(_:)) {
      menuItem.state = showsStatusBar ? .on : .off
      return true
    }

    guard
      menuItem.action == #selector(performEditorMenuCommand(_:)),
      let command = EVMenuCommand(rawValue: menuItem.tag)
    else { return true }

    if command == .newWindowForDocument {
      return document != nil
    }

    let presentation = commandSurface.presentation(for: command)
    menuItem.state = presentation.state
    if let title = presentation.title {
      menuItem.title = title
    }
    return presentation.isEnabled
  }
}

// MARK: Diff mode

@MainActor
extension EVDocumentWindowController: EVDiffModeHost {
  var diffWindow: NSWindow? { window }
  var diffPanes: [EVDocumentContentViewController] { isClosed ? [] : paneContainer.panes }
  var diffActivePane: EVDocumentContentViewController? { isClosed ? nil : documentContentController }
  var diffOpenPanelPresenter: ((NSOpenPanel, NSWindow, @escaping @MainActor (URL?) -> Void) -> Void)? {
    openFilePanelPresenter
  }

  func openDiffFile(_ url: URL, beforeReplacing: @escaping @MainActor () -> Bool,
                    completion: @escaping @MainActor (Error?) -> Void) {
    let pane = documentContentController
    guard let document = pane.document else { completion(EVDocumentHostError.staleRequest); return }
    let expected = document.editorBackend.persistenceState
    // The caller has already shown the window-close save review. Moving
    // between diff files opens through the argument commands' boundary.
    loadArgumentDocument(url) { [weak self] opened, error in
      guard let self, let opened else { completion(error ?? EVDocumentHostError.unsupportedRequest); return }
      guard self.canReplace(pane, document: document, expected: expected), beforeReplacing() else {
        self.closeIfUnrepresented(opened)
        completion(EVDocumentHostError.staleRequest)
        return
      }
      self.replacePane(pane, with: opened)
      self.closeIfUnrepresented(document)
      completion(nil)
    }
  }

  func unsavedDocumentURLs() -> [URL] {
    NSDocumentController.shared.documents.compactMap { document in
      guard let document = document as? EVDocument, let url = document.fileURL,
        document.editorBackend.persistenceState.isDirty else { return nil }
      return url
    }
  }

  private var diffController: EVDiffModeController {
    if let diffMode { return diffMode }
    let controller = EVDiffModeController(host: self, configuration: configuration)
    diffMode = controller
    return controller
  }

  fileprivate func editorMenuCommandDidPerform(
    _ command: EVMenuCommand, on surface: any EVEditorSurface, in pane: EVDocumentContentViewController
  ) {
    guard command == .showLineNumbers else { return }
    if diffMode?.isEnabled == true {
      diffMode?.lineNumbersDidToggle(on: surface)
    } else if pane.document?.editorBackend.sourceFormat == .code,
      let shows = (surface as? any EVDiffLineNumberControlling)?.showsLineNumbers {
      try? configuration.setCodeLineNumbers(shows)
    }
  }

  /// Diff Mode has its own window. From an ordinary window, compare the
  /// active document in a new diff window, leaving this one unchanged; in a
  /// diff window, show the document in an ordinary window and close this one.
  @objc func toggleDiffMode(_ sender: Any?) {
    if isDiffWindow { leaveDiffWindow() } else { openDiffWindow() }
  }

  private func openDiffWindow() {
    guard let document = activeDocument else { return }
    let target = EVDocumentWindowController(document: document,
                                            editorSurface: document.editorBackend.makeEditorSurface(display: .diff),
                                            diffPlacement: Self.folderDiffPlacement)
    target.markOpensForDiff()
    target.argumentDocumentOpener = argumentDocumentOpener
    document.addWindowController(target)
    target.showWindow(nil)
    target.diffController.enable()
  }

  /// Show the active document in an ordinary window, reusing one that already
  /// shows it, then close this diff window. The buffer stays open throughout,
  /// so unsaved edits carry over without a save review.
  private func leaveDiffWindow() {
    if let document = activeDocument { Self.showOrdinaryWindow(of: document) }
    diffMode?.disable()
    window?.performClose(nil)
  }

  /// Bring forward an ordinary (not diff) window that shows `document`,
  /// focusing its pane, or open one when none does.
  static func showOrdinaryWindow(of document: EVDocument) {
    for controller in instances.compactMap(\.value) where !controller.isClosed && !controller.isDiffWindow {
      guard let index = controller.paneContainer.panes.firstIndex(where: { $0.document === document }) else { continue }
      controller.showWindow(nil)
      controller.paneContainer.focusPane(at: index)
      controller.updateActiveDocumentChrome()
      return
    }
    document.showAdditionalWindow()
  }
  @objc func toggleDiffAlignSides(_ sender: Any?) { diffMode?.toggleAlign() }
  @objc func toggleDiffCollapseContext(_ sender: Any?) { diffMode?.toggleCollapse() }
  @objc func refreshDiffBase(_ sender: Any?) { diffMode?.refresh() }
  @objc func chooseDiffBaseFile(_ sender: Any?) { diffMode?.chooseBaseFile() }
  @objc func showPreviousDiffFile(_ sender: Any?) { diffMode?.moveFile(by: -1) }
  @objc func showNextDiffFile(_ sender: Any?) { diffMode?.moveFile(by: 1) }
  @objc func diffCurrentDirectory(_ sender: Any?) {
    Self.diffFolder(URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true), from: self)
  }
  @objc func chooseDiffFolder(_ sender: Any?) { Self.chooseDiffFolder(from: self) }

  /// Pick a folder for "Diff Folder…", as a sheet when a window is given.
  static func chooseDiffFolder(from controller: EVDocumentWindowController?) {
    let panel = NSOpenPanel()
    panel.canChooseFiles = false
    panel.canChooseDirectories = true
    panel.allowsMultipleSelection = false
    panel.showsHiddenFiles = true
    panel.directoryURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
    panel.message = "Choose a folder to compare with its git index"
    panel.prompt = "Diff"
    let selected: @MainActor (URL?) -> Void = { url in
      guard let url else { return }
      diffFolder(url, from: controller)
    }
    if let controller, let window = controller.window {
      if let presenter = controller.openFilePanelPresenter {
        presenter(panel, window, selected)
      } else {
        panel.beginSheetModal(for: window) { response in
          MainActor.assumeIsolated { selected(response == .OK ? panel.url : nil) }
        }
      }
    } else {
      selected(panel.runModal() == .OK ? panel.url : nil)
    }
  }

  /// Show the first file `git diff -- <folder>` lists in diff mode, in the
  /// given window when it holds only a pristine untitled buffer, otherwise
  /// in a new window. A folder without changes shows an empty buffer there;
  /// a folder outside a repository is reported instead. Cancelling the
  /// returned task before the listing finishes opens and reports nothing,
  /// and completes with `nil`.
  @discardableResult
  static func diffFolder(_ folder: URL, from controller: EVDocumentWindowController?,
                         completion: ((EVDocumentWindowController?) -> Void)? = nil) -> Task<Void, Never> {
    let unsaved = (controller?.unsavedDocumentURLs() ?? []).map(\.path)
    return Task { @MainActor in
      let listed = await EVDiffBaseLoader.listFiles(.unstaged, path: folder.path, unsaved: unsaved, scope: folder.path)
      guard !Task.isCancelled else {
        completion?(nil)
        return
      }
      let files: [EVDiffFile]
      switch listed {
      case let .success(found):
        files = found
      case let .failure(error):
        reportDiffFolder(error.localizedDescription, in: controller)
        completion?(nil)
        return
      }
      let replacement = controller?.captureUntitledReplacement()
      guard let first = files.first else {
        let target: EVDocumentWindowController
        if let controller, replacement != nil {
          // The pristine untitled buffer is already the empty buffer to show.
          target = controller
          controller.markOpensForDiff()
          controller.adoptDiffPlacement()
        } else {
          guard let empty = folderDiffUntitledDocument?() ?? makeUntitledDiffDocument() else {
            reportDiffFolder("There are no changes in \(folder.path).", in: controller)
            completion?(nil)
            return
          }
          target = EVDocumentWindowController(document: empty, editorSurface: empty.editorBackend.makeEditorSurface(display: .diff),
                                              diffPlacement: folderDiffPlacement)
          target.argumentDocumentOpener = controller?.argumentDocumentOpener
          target.markOpensForDiff()
          empty.addWindowController(target)
          target.showWindow(nil)
        }
        target.diffController.enable(folder: folder, files: [])
        folderDiffDidOpen?(target)
        completion?(target)
        return
      }
      let open = folderDiffDocumentOpener ?? { url, completion in openPaneDocument(url, completion: completion) }
      open(first.url) { opened, error in
        guard let opened else {
          reportDiffFolder(error?.localizedDescription ?? "The file could not be opened.", in: controller)
          completion?(nil)
          return
        }
        let target: EVDocumentWindowController
        // The reused window becomes a diff window before its new pane exists,
        // so that pane is Code from its first frame.
        let wasDiffWindow = controller?.opensForDiff ?? false
        if replacement != nil { controller?.markOpensForDiff() }
        if let controller, let replacement, replacement.install(opened) {
          target = controller
          controller.adoptDiffPlacement()
        } else {
          if replacement != nil { controller?.opensForDiff = wasDiffWindow }
          target = EVDocumentWindowController(document: opened, editorSurface: opened.editorBackend.makeEditorSurface(display: .diff),
                                              diffPlacement: folderDiffPlacement)
          target.argumentDocumentOpener = controller?.argumentDocumentOpener
          target.markOpensForDiff()
          opened.addWindowController(target)
          target.showWindow(nil)
        }
        target.diffController.enable(folder: folder, files: files)
        folderDiffDidOpen?(target)
        completion?(target)
      }
    }
  }

  static func openFileDiff(_ document: EVDocument, initialLine: UInt64?) -> EVDocumentWindowController {
    let target = EVDocumentWindowController(document: document, editorSurface: document.editorBackend.makeEditorSurface(display: .diff),
                                            diffPlacement: folderDiffPlacement)
    target.markOpensForDiff()
    document.addWindowController(target)
    target.showWindow(nil)
    target.diffController.enable(singleFile: true)
    if let initialLine { target.editorSurface.goToLine(initialLine) }
    return target
  }

  /// A reused placeholder window becomes a diff window: give it the last
  /// diff window's size, keeping its top-left corner on screen.
  func adoptDiffPlacement() {
    guard let window, !window.styleMask.contains(.fullScreen), !window.isMiniaturized,
          let remembered = diffPlacement.rememberedFrame else { return }
    var frame = EVDocumentWindowPlacement.includingGutter(remembered, width: placementGutterWidth)
    frame.origin = NSPoint(x: window.frame.minX, y: window.frame.maxY - frame.height)
    window.setFrame(diffPlacement.fittedFrame(frame), display: true)
  }

  private static func makeUntitledDiffDocument() -> EVDocument? {
    guard let document = try? NSDocumentController.shared.makeUntitledDocument(ofType: EVDocument.plainTextType)
      as? EVDocument else { return nil }
    NSDocumentController.shared.addDocument(document)
    return document
  }

  /// Tests inject the placement new diff windows use.
  static var folderDiffPlacement: EVDocumentWindowPlacement?
  /// Tests make the empty buffer a folder without changes shows.
  static var folderDiffUntitledDocument: (() -> EVDocument?)?
  /// Tests open documents without registered document types.
  static var folderDiffDocumentOpener: ((URL, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void)?
  /// Tests observe the window a folder diff opened in.
  static var folderDiffDidOpen: ((EVDocumentWindowController) -> Void)?
  /// Tests capture folder-diff reports instead of showing an alert.
  static var folderDiffReport: ((String) -> Void)?

  private static func reportDiffFolder(_ message: String, in controller: EVDocumentWindowController?) {
    if let folderDiffReport {
      folderDiffReport(message)
    } else if let controller, !controller.isClosed {
      controller.editorSurface.showDocumentMessage(message)
    } else {
      let alert = NSAlert()
      alert.messageText = "Can't diff the folder"
      alert.informativeText = message
      alert.runModal()
    }
  }
}

@MainActor
extension EVDocumentWindowController: NSMenuItemValidation {
  public func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
    let enabled = diffMode?.isEnabled == true
    switch menuItem.action {
    case #selector(toggleDiffMode(_:)):
      menuItem.state = enabled ? .on : .off
      return activeDocument != nil
    case #selector(toggleDiffAlignSides(_:)):
      menuItem.state = (diffMode?.isAligned ?? configuration.diffAlignSides) ? .on : .off
      return enabled
    case #selector(toggleDiffCollapseContext(_:)):
      menuItem.state = (diffMode?.isCollapsed ?? configuration.diffCollapseContext) ? .on : .off
      return enabled
    case #selector(refreshDiffBase(_:)), #selector(chooseDiffBaseFile(_:)):
      return enabled
    case #selector(showPreviousDiffFile(_:)), #selector(showNextDiffFile(_:)):
      return enabled && !(diffMode?.files.isEmpty ?? true)
    default:
      return true
    }
  }
}
