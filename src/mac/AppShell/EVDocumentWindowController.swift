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
  private let paneContainer: EVPaneContainer
  private static var instances: [EVWeakDocumentWindow] = []
  static var hasOpenDocumentWindows: Bool {
    instances.contains { $0.value.map { !$0.isClosed } ?? false }
  }
  var commandDidCloseWindow: () -> Void = {
    (NSApplication.shared.delegate as? EVApplicationDelegate)?.terminateAfterCommandClose()
  }
  private var isClosed = false
  private var closeQueue: [EVDocument] = []
  private var closeReviewCompletion: ((Bool) -> Void)?
  private var closingAfterReview = false
  private var hasPresentedInitialWindow = false
  private var isPerformingDocumentHostEffect = false
  private var sourceDepth: UInt32 = 0
  private var sourceCommandCount = 0
  private let placement: EVDocumentWindowPlacement
  private var initialWindowFrame: NSRect?
  private lazy var formattingToolbar = EVFormattingToolbarChrome(
    window: window!, configuration: .shared)

  /// Injectable I/O boundary for launch and argument-list opens.
  var argumentDocumentOpener: ((URL, String?, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void)?

  var currentGeometry: EVDocumentWindowGeometry? {
    guard let window, synchronizeContentFrame(of: window) != nil else { return nil }
    return documentContentController.geometry(in: documentContentController.view)
  }

  public init(document: EVDocument, editorSurface: any EVEditorSurface, placement: EVDocumentWindowPlacement? = nil) {
    self.placement = placement ?? .shared
    let firstPane = EVDocumentContentViewController(editorSurface: editorSurface)
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

  var paneCount: Int { paneContainer.panes.count }

  /// `CTRL-W` window effects. Pane order, focus, and heights live in the pane
  /// container; a request from a pane that is no longer in this window is
  /// ignored rather than applied to someone else's panes.
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
    updateActiveDocumentChrome()
    var seen = Set<ObjectIdentifier>()
    for pane in paneContainer.panes {
      guard let document = pane.document, seen.insert(ObjectIdentifier(document)).inserted else { continue }
      document.requestExternalFileReview()
    }
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
    if hasPresentedInitialWindow, let window { placement.record(window: window) }
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
      placement.record(window: window)
    }
    _ = synchronizeContentFrame(of: window)
    window.makeFirstResponder(editorSurface.viewController.view)
    window.makeKeyAndOrderFront(sender)
  }

  private func prepareInitialFrame(of window: NSWindow) {
    if initialWindowFrame == nil {
      initialWindowFrame = placement.initialFrame(for: window, defaultContentSize: Self.initialContentSize)
    }
    if let initialWindowFrame { window.setFrame(placement.fittedFrame(initialWindowFrame), display: false) }
    _ = synchronizeContentFrame(of: window)
  }

  public func windowDidMove(_ notification: Notification) { recordWindowPlacement() }
  public func windowDidResize(_ notification: Notification) { recordWindowPlacement() }

  private func recordWindowPlacement() {
    guard hasPresentedInitialWindow, !isClosed, let window else { return }
    placement.record(window: window)
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
  /// Startup and forwarded command lines install the same argument list.
  /// Already represented files keep their windows and panes; only new files
  /// occupy this initially hidden window. Later arguments stay lazy unless
  /// requested by `-o`, including when the first file was already open.
  public func openArgumentList(
    _ urls: [URL], splitCount: Int?, initialLine: UInt64?,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard !isClosed, !isPerformingDocumentHostEffect,
      let original = activeDocument, Self.isPristineUntitled(original) else {
      completion(.failure(EVDocumentHostError.operationAlreadyInProgress)); return
    }
    let pane = documentContentController
    let state = original.editorBackend.persistenceState
    let list = EVArgumentList(urls)
    let argumentLimit = splitCount.map { $0 == 0 ? max(1, urls.count) : max(1, $0) } ?? 1
    var installedNewPane = false
    var firstTarget: (EVDocumentWindowController, EVDocumentContentViewController)?
    isPerformingDocumentHostEffect = true

    func finish(_ result: Result<String?, Error>) {
      if !installedNewPane && firstTarget != nil {
        // Every requested file was already shown. Retire the unused blank
        // without ever presenting another document window.
        close()
      } else if !isClosed {
        showWindow(nil)
        paneContainer.focusPane(at: 0)
        updateActiveDocumentChrome()
      }
      if let (controller, target) = firstTarget, !controller.isClosed,
        let index = controller.paneContainer.panes.firstIndex(where: { $0 === target }) {
        controller.paneContainer.focusPane(at: index)
        controller.updateActiveDocumentChrome()
        if controller.window?.isMiniaturized == true { controller.window?.deminiaturize(nil) }
        controller.showWindow(nil)
        controller.window?.makeKeyAndOrderFront(nil)
      }
      finishDocumentHostEffect(result, completion: completion)
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
      guard index < argumentLimit else { finish(.success(nil)); return }
      // Reusing another window or an earlier argument consumes no space here.
      // Once this window is full, leave the next new target lazy without even
      // reading it, while allowing already represented arguments to be reused.
      if installedNewPane && paneCount >= launchPaneCapacity() {
        guard list.urls.indices.contains(index),
          let existing = EVDocumentIdentity.existingDocument(at: list.urls[index]),
          Self.windowShowing(document: existing) != nil else {
          finish(.success(nil)); return
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
        if opened !== original, let controller = Self.windowShowing(document: opened)?.windowController
          as? EVDocumentWindowController,
          let existingPane = controller.paneContainer.panes.first(where: { $0.document === opened }) {
          targetController = controller
          targetPane = existingPane
        } else {
          guard installedNewPane || (canReplace(pane, document: original, expected: state)
            && Self.isPristineUntitled(original)) else {
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
            addPane(document: opened)
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

      if list.urls.indices.contains(index) {
        loadArgumentDocument(list.urls[index], fallback: nil, completion: installed)
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

  private func launchPaneCapacity() -> Int {
    let row = max(1, editorSurface.visualRowHeight ?? EVStatusBarView.preferredHeight)
    let minimum = row + documentContentController.statusBarHeight + 1
    return max(1, Int((window?.contentLayoutRect.height ?? 680) / minimum))
  }

  fileprivate func loadArgumentDocument(
    _ url: URL, fallback: String?, completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    if let argumentDocumentOpener { argumentDocumentOpener(url, fallback, completion) }
    else { openPaneDocument(url, fallback: fallback, completion: completion) }
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
    guard let list = pane.argumentList else { completion(.failure(EVArgumentListError.empty)); return }
    guard let resolve = EVArgumentListPolicy.resolve else {
      completion(.failure(EVDocumentHostError.unsupportedRequest)); return
    }
    let expected = document.editorBackend.persistenceState
    let currentIndex = list.index(of: document.fileURL, preferring: pane.argumentIndex)
    let resolved = resolve(list.urls.count, currentIndex, pane.argumentIndex, navigation)

    func openTarget() {
      do {
        guard canReplace(pane, document: document, expected: expected) else { throw EVDocumentHostError.staleRequest }
        let index = try resolved.get()
        guard list.urls.indices.contains(index) else { throw EVArgumentListError.invalidIndex }
        let url = list.urls[index]
        let sameDocument = document.fileURL.map { EVDocumentIdentity.sameFile($0, url) } ?? false
        guard sameDocument || request.force || !document.editorBackend.persistenceState.isDirty
          || hasOtherView(of: document, excluding: pane) else { throw EVDocumentHostError.documentModified }
        if sameDocument {
          pane.argumentIndex = index
          if let line = navigation.line { pane.editorSurface.goToLine(line == 0 ? UInt64.max : line) }
          completion(.success(nil)); return
        }
        loadArgumentDocument(url, fallback: nil) { opened, error in
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
          self.documentContentController.argumentIndex = index
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

  fileprivate func finishDocumentHostEffect(
    _ result: Result<String?, Error>,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
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

  fileprivate func performDocumentHostRequest(
    _ request: EVDocumentHostRequest,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
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

    switch request.kind {
    case .read:
      do {
        guard let path = request.path ?? document.fileURL?.path,
              let url = resolvedFileURL(path, relativeTo: nil),
              let after = request.readAfterLine,
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
              try pane.editorSurface.insertFileContents(bytes, after: after, expected: persistence)
              completion(.success("\(url.path) read"))
            } catch { completion(.failure(error)) }
          }
        }
      } catch { completion(.failure(error)) }
    case .source:
      sourceCommands(request.path, completion: completion)
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
    case .split:
      split(document, path: request.path, initialHeightRows: request.initialHeightRows, completion: completion)
    case .newPane:
      do {
        let next = EVDocument()
        try next.read(from: Data(), ofType: EVDocument.plainTextType)
        next.fileType = EVDocument.plainTextType
        NSDocumentController.shared.addDocument(next)
        addPane(document: next, initialHeightRows: request.initialHeightRows)
        completion(.success(nil))
      } catch {
        completion(.failure(error))
      }
    case .write:
      if request.path != nil || request.hardLineRange != nil {
        writeAlternate(document, request: request, closeAfter: false, completion: completion)
        return
      }
      guard request.path == nil, request.hardLineRange == nil else {
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
      if request.path != nil || request.hardLineRange != nil {
        writeAlternate(document, request: request, closeAfter: true, completion: completion)
        return
      }
      guard request.path == nil, request.hardLineRange == nil else {
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
      completion(.success(FileManager.default.currentDirectoryPath))

    case .editNewWindow:
      guard let path = request.path ?? document.fileURL?.path,
        let destination = resolvedFileURL(path, relativeTo: nil) else {
        completion(.failure(EVDocumentHostError.noDocumentURL))
        return
      }
      let sourcePane = documentContentController
      openPaneDocument(destination, fallback: document.fileType) { opened, error in
        guard let opened else { completion(.failure(error ?? EVDocumentHostError.unsupportedRequest)); return }
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
        || (!reloadsCurrent && hasOtherView(of: document, excluding: documentContentController)) else {
        completion(.failure(EVDocumentHostError.documentModified))
        return
      }
      edit(document, path: request.path, force: request.force, completion: completion)

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

  private func sourceCommands(_ path: String?, completion: @escaping @MainActor (Result<String?, Error>) -> Void) {
    do {
      guard let path, let url = resolvedFileURL(path, relativeTo: nil) else { throw EVDocumentHostError.invalidPath(path ?? "") }
      guard sourceDepth < UInt32(VIEM_SOURCE_MAX_DEPTH) else {
        throw EVSourceCommandError(path: url.path, line: 0, message: "Source nesting limit exceeded.")
      }
      let file = try FileHandle(forReadingFrom: url)
      defer { try? file.close() }
      let data = try file.read(upToCount: Int(VIEM_SOURCE_MAX_BYTES) + 1) ?? Data()
      guard data.count <= Int(VIEM_SOURCE_MAX_BYTES), let text = String(data: data, encoding: .utf8) else {
        throw EVSourceCommandError(path: url.path, line: 0, message: "Source files must be UTF-8 and at most 1 MiB.")
      }
      if sourceDepth == 0 { sourceCommandCount = 0 }
      sourceDepth += 1
      let depth = sourceDepth
      let lines = text.components(separatedBy: "\n")
      var targetPane = documentContentController
      func finish(_ result: Result<String?, Error>) { self.sourceDepth -= 1; completion(result) }
      func next(_ index: Int) {
        guard index < lines.count else { finish(.success(nil)); return }
        self.sourceCommandCount += 1
        guard !self.isClosed, self.sourceCommandCount <= Int(VIEM_SOURCE_MAX_COMMANDS) else {
          finish(.failure(EVSourceCommandError(path: url.path, line: index + 1, message: "Source command limit exceeded or window closed.")))
          return
        }
        do {
          guard let targetIndex = self.paneContainer.panes.firstIndex(where: { $0 === targetPane }) else {
            throw EVDocumentHostError.staleRequest
          }
          // A user's focus change while a sourced host request is pending
          // must not redirect the remaining script into an unrelated buffer.
          self.paneContainer.focusPane(at: targetIndex)
          let requests = try targetPane.editorSurface.executeSourcedLine(lines[index], depth: depth)
          targetPane = self.documentContentController
          let navigates = requests.contains { [.edit, .new, .split, .newPane, .navigateArgument, .quit].contains($0.kind) }

          self.performSourcedHostRequests(requests[...]) { result in
            switch result {
            case .success:
              if navigates { targetPane = self.documentContentController }
              DispatchQueue.main.async { next(index + 1) }
            case .failure(let error):
              finish(.failure(EVSourceCommandError(path: url.path, line: index + 1, message: error.localizedDescription)))
            }
          }
        } catch {
          finish(.failure(EVSourceCommandError(path: url.path, line: index + 1, message: error.localizedDescription)))
        }
      }
      next(0)
    } catch { completion(.failure(error)) }
  }

  private func performSourcedHostRequests(_ requests: ArraySlice<EVDocumentHostRequest>, completion: @escaping @MainActor (Result<Void, Error>) -> Void) {
    guard let request = requests.first else { completion(.success(())); return }
    performDocumentHostRequest(request) { result in
      switch result {
      case .success(let message):
        if let message { self.editorSurface.showDocumentMessage(message) }
        self.performSourcedHostRequests(requests.dropFirst(), completion: completion)
      case .failure(let error): completion(.failure(error))
      }
    }
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
      if let range = request.hardLineRange {
        snapshot = try document.editorBackend.nativeSaveSnapshot(typeName: document.fileType ?? EVDocument.plainTextType, hardLineRange: range)
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
    _ source: EVDocument, path: String?, initialHeightRows: Int? = nil,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard let path else {
      addPane(document: source, initialHeightRows: initialHeightRows)
      completion(.success(nil))
      return
    }
    guard let url = resolvedFileURL(path, relativeTo: source.fileURL) else {
      completion(.failure(EVDocumentHostError.invalidPath(path)))
      return
    }
    openPaneDocument(url, fallback: source.fileType) { [weak self] opened, error in
      if let opened {
        self?.addPane(document: opened, initialHeightRows: initialHeightRows)
        completion(.success(nil))
      } else {
        completion(.failure(error ?? EVDocumentHostError.unsupportedRequest))
      }
    }
  }

  fileprivate func openPaneDocument(
    _ url: URL, fallback: String?, completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    if !FileManager.default.fileExists(atPath: url.path),
      EVDocumentIdentity.existingDocument(at: url) == nil
    {
      do {
        let document = EVDocument()
        let type = documentType(for: url, fallback: fallback)
        try document.read(
          from: Data(),
          ofType: EVDocument.defaultOpeningType(for: url, nativeType: type)
        )
        document.fileURL = EVDocumentIdentity.canonicalURL(url)
        document.fileType = type
        document.configureRecovery(for: EVDocumentIdentity.canonicalURL(url))
        NSDocumentController.shared.addDocument(document)
        completion(document, nil)
      } catch { completion(nil, error) }
    } else {
      EVDocumentIdentity.open(url, display: false, completion: completion)
    }
  }

  fileprivate func edit(
    _ document: EVDocument, path: String?, force: Bool = false,
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
    let pane = documentContentController
    let state = document.editorBackend.persistenceState
    openPaneDocument(url, fallback: document.fileType) { [weak self] opened, error in
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

  fileprivate func makePane(document: EVDocument) -> EVDocumentContentViewController {
    let surface = document.editorBackend.makeEditorSurface()
    let pane = EVDocumentContentViewController(editorSurface: surface)
    pane.document = document
    let previous = documentContentController
    pane.argumentList = previous.argumentList
    pane.argumentIndex = previous.argumentList?.index(of: previous.document?.fileURL,
      preferring: previous.argumentIndex) ?? previous.argumentIndex
    if let matched = pane.argumentList?.index(of: document.fileURL, preferring: pane.argumentIndex) {
      pane.argumentIndex = matched
    }
    (surface as? any EVDocumentHostAttachable)?.documentHostEffectHandler = self
    return pane
  }

  fileprivate func addPane(document: EVDocument, initialHeightRows: Int? = nil) {
    paneContainer.insert(makePane(document: document))
    if let rows = initialHeightRows {
      paneContainer.perform(.setHeight(rows: rows))
    }
    updateActiveDocumentChrome()
  }

  fileprivate func replaceActivePane(with document: EVDocument) {
    replacePane(documentContentController, with: document)
  }

  fileprivate func replacePane(_ old: EVDocumentContentViewController, with document: EVDocument) {
    let next = makePane(document: document)
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
  weak var document: EVDocument?
  var argumentList: EVArgumentList?
  var argumentIndex: Int?

  let editorSurface: any EVEditorSurface
  private let statusBar = EVStatusBarView()
  private var showsStatusBar: Bool

  init(editorSurface: any EVEditorSurface) {
    self.editorSurface = editorSurface
    showsStatusBar = EVConfigurationStore.shared.showStatusBar
    super.init(nibName: nil, bundle: nil)

    editorSurface.statusBarStateDidChange = { [weak self] state in
      guard let self else { return }
      let outputHadFocus = self.statusBar.isCommandOutputFocused
      self.statusBar.apply(state)
      if outputHadFocus, state.commandOutput == nil || state.commandLine != nil {
        self.viewIfLoaded?.window?.makeFirstResponder(self.editorSurface.viewController.view)
      }
      // The command line lives in the status line, so a hidden status line
      // still has to appear while one is active.
      let needed = self.showsStatusBar || state.commandLine != nil || state.commandOutput != nil || state.requiresInteraction
      if self.statusBar.isHidden == needed {
        self.statusBar.isHidden = !needed
        self.layoutContent()
      }
      (self.viewIfLoaded?.window?.windowController as? EVDocumentWindowController)?
        .updateActiveDocumentChrome()
    }
    statusBar.preferredHeightDidChange = { [weak self] in self?.layoutContent() }
    statusBar.optionDidChange = { [weak self] option in
      guard let self else { return }
      self.editorSurface.perform(statusOption: option)
      self.statusBar.apply(self.editorSurface.statusBarState)
      self.view.window?.makeFirstResponder(self.editorSurface.viewController.view)
    }
    // The command line is drawn by the status line, so pointer selection in it
    // is reported from there and routed back to this pane's surface.
    statusBar.commandLineDidSelect = { [weak self] offset, extending in
      guard let self else { return }
      self.editorSurface.selectCommandLine(atUTF8Offset: offset, extending: extending)
    }
    statusBar.commandOutputDidDismiss = { [weak self] in
      self?.editorSurface.dismissCommandOutput()
    }
    statusBar.commandOutputDidReceiveKey = { [weak self] event in
      self?.editorSurface.handleStatusMessageKey(event)
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  override func loadView() {
    let root = NSView()
    // Programmatic content views do not receive the nib loader's default
    // width/height autoresizing mask. Keep the controller root matched to
    // the window's content rect as the window is shown and resized.
    root.autoresizingMask = [.width, .height]

    addChild(editorSurface.viewController)
    let editorView = editorSurface.viewController.view
    editorView.translatesAutoresizingMaskIntoConstraints = true
    editorView.autoresizingMask = [.width, .height]
    statusBar.translatesAutoresizingMaskIntoConstraints = true
    statusBar.autoresizingMask = [.width, .maxYMargin]
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

  var statusBarHeight: CGFloat { statusBar.isHidden ? 0 : EVStatusBarView.preferredHeight }

  func layoutContent() {
    loadViewIfNeeded()
    let bounds = view.bounds
    let statusHeight = statusBarHeight
    let editorView = editorSurface.viewController.view
    statusBar.frame = NSRect(
      x: bounds.minX,
      y: bounds.minY,
      width: bounds.width,
      height: statusHeight
    )
    editorView.frame = NSRect(
      x: bounds.minX,
      y: bounds.minY + statusHeight,
      width: bounds.width,
      height: max(0, bounds.height - statusHeight)
    )
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
      editorSurface.perform(menuCommand: command, sender: sender)
    }
  }

  @objc func toggleStatusBar(_ sender: Any?) {
    showsStatusBar.toggle()
    statusBar.isHidden = !showsStatusBar && editorSurface.statusBarState.commandLine == nil
      && editorSurface.statusBarState.commandOutput == nil
      && !editorSurface.statusBarState.requiresInteraction
    layoutContent()
    try? EVConfigurationStore.shared.setShowStatusBar(showsStatusBar)
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

    let presentation = editorSurface.presentation(for: command)
    menuItem.state = presentation.state
    if let title = presentation.title {
      menuItem.title = title
    }
    return presentation.isEnabled
  }
}

private struct EVSourceCommandError: LocalizedError {
  let path: String
  let line: Int
  let message: String
  var errorDescription: String? { "\(path):\(line): \(message)" }
}
