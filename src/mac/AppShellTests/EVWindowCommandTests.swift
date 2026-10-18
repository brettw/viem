import AppKit
import CViemCore
import XCTest

@testable import ViemAppShell

/// `CTRL-W` window effects against real stacked panes: focus order, pane
/// reordering, and heights measured in default paragraph lines.
@MainActor
final class EVWindowCommandTests: XCTestCase {
  private final class EditorTestView: NSView {
    override var acceptsFirstResponder: Bool { true }
  }

  /// SwiftPM can run behind another foreground application. Model key focus
  /// as the editor fixtures do while preserving native event dispatch.
  private final class PointerTestWindow: NSWindow {
    private var keyForTesting = false
    override var isKeyWindow: Bool { keyForTesting }
    // Match EVDocumentWindow: SwiftPM's host can have no assigned screen while
    // ordering, when NSWindow's default would collapse to its fitting minimum.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
      guard let screen else { return frameRect }
      return super.constrainFrameRect(frameRect, to: screen)
    }
    func makeKeyForTesting() {
      guard !keyForTesting else { return }
      keyForTesting = true
      NotificationCenter.default.post(name: NSWindow.didBecomeKeyNotification, object: self)
    }
  }

  private final class Surface: EVEditorSurface {
    let viewController: NSViewController = {
      let controller = NSViewController()
      controller.view = EditorTestView(frame: NSRect(x: 0, y: 0, width: 920, height: 655))
      return controller
    }()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    var rowHeight: CGFloat = 20
    var lineNumberGutterWidth: CGFloat = 0
    var lineNumberGutterWidthWillChange: ((CGFloat, CGFloat) -> Void)?
    var defaultLineHeight: CGFloat? { rowHeight }
    var statusClicks = 0
    func perform(statusOption _: EVStatusBarOption) { statusClicks += 1 }
    func perform(menuCommand _: EVMenuCommand, sender _: Any?) {}
    func presentation(for _: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
  }

  func testWindowRestorationAndSynchronousGutterResizingPreserveSavedContentWidth() throws {
    let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 1000)
    let original = NSRect(x: screen.minX + 100, y: screen.minY + 100,
      width: min(700, screen.width - 200), height: min(500, screen.height - 200))
    var saved: NSRect? = original
    for gutter in [CGFloat(31), 50, 0] {
      let backend = Backend()
      let document = EVDocument(editorBackend: backend)
      let surface = Surface()
      surface.lineNumberGutterWidth = gutter
      let placement = EVDocumentWindowPlacement(loadFrame: { saved },
        saveFrame: { saved = $0 }, visibleScreens: { [screen] })
      let controller = EVDocumentWindowController(document: document,
        editorSurface: surface, placement: placement)
      document.addWindowController(controller)
      controller.showWindow(nil)
      let window = try XCTUnwrap(controller.window)
      XCTAssertEqual(window.frame.width - gutter, original.width, accuracy: 1)
      XCTAssertEqual(window.frame.minX + gutter, original.minX, accuracy: 1)
      let grown = gutter + 15
      surface.lineNumberGutterWidthWillChange?(gutter, grown)
      if window.screen == nil {
        // SwiftPM can run without a Window Server screen. Deliver the same
        // native resize against the injected desktop before publishing width.
        window.setFrame(EVDocumentWindowPlacement.frameForGutterChange(window.frame,
          delta: grown - gutter, screen: screen, minimumWidth: window.minSize.width), display: false)
        controller.windowDidResize(Notification(name: NSWindow.didResizeNotification, object: window))
      }
      // This callback resizes the native window before the surface publishes
      // its new width, just like a real numeral-digit threshold change.
      XCTAssertEqual(try XCTUnwrap(saved).width, original.width, accuracy: 1)
      surface.lineNumberGutterWidth = grown
      window.close()
      XCTAssertEqual(try XCTUnwrap(saved).width, original.width, accuracy: 1)
      XCTAssertEqual(try XCTUnwrap(saved).minX, original.minX, accuracy: 1)
    }
  }

  private final class Backend: EVDocumentBackend {
    var sourceDidChange: (() -> Void)?
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
    var persistenceState = EVDocumentPersistenceState()
    var sourceFormat: EVSourceFormat = .plainText
    var surfaces: [Surface] = []
    func makeEditorSurface() -> any EVEditorSurface {
      let surface = Surface()
      surfaces.append(surface)
      return surface
    }
    func read(source _: Data, typeName _: String) throws {}
    func serializedSource(typeName _: String) throws -> Data { Data() }
    func nativeSaveSnapshot(typeName _: String) throws -> EVDocumentSaveSnapshot {
      EVDocumentSaveSnapshot(data: Data(), documentID: 0, documentRevision: 0)
    }
    func acknowledgeNativeSave(_: EVDocumentSaveSnapshot) throws {}
    func setReadOnly(_: Bool) throws {}
  }

  /// A window with `count` stacked panes, focused on the top one.
  private func makeWindow(panes count: Int) throws -> (
    EVDocumentWindowController, Backend, EVDocument
  ) {
    let backend = Backend()
    let document = EVDocument(editorBackend: backend)
    document.makeWindowControllers()
    let controller = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
    controller.showWindow(nil)
    // Height assertions need the declared fixture size, independent of a
    // previously tested window's persisted placement.
    controller.window?.setContentSize(EVDocumentWindowController.initialContentSize)
    controller.window?.contentView?.layoutSubtreeIfNeeded()
    let state = backend.persistenceState
    for _ in 1 ..< count {
      var result: Result<String?, Error>?
      controller.perform(documentHostRequests: [
        EVDocumentHostRequest(
          kind: .split, documentID: state.documentID, documentRevision: state.documentRevision)
      ]) { result = $0 }
      _ = try XCTUnwrap(result).get()
    }
    XCTAssertEqual(controller.paneCount, count)
    controller.perform(windowRequests: [.focusTop], from: controller.editorSurface)
    return (controller, backend, document)
  }

  /// Pane surfaces in top-to-bottom screen order.
  private func stackedSurfaces(_ controller: EVDocumentWindowController, _ backend: Backend)
    -> [Surface]
  {
    backend.surfaces.sorted { first, second in
      let a = first.viewController.view.convert(first.viewController.view.bounds, to: nil)
      let b = second.viewController.view.convert(second.viewController.view.bounds, to: nil)
      return a.maxY > b.maxY
    }
  }

  private func focusedIndex(_ controller: EVDocumentWindowController, _ backend: Backend) -> Int? {
    let order = stackedSurfaces(controller, backend)
    return order.firstIndex { $0 === (controller.editorSurface as? Surface) }
  }

  func testInteractiveSplitFocusesNewUpperOrLeftPaneAndAppliesCount() throws {
    for vertical in [false, true] {
      let (controller, backend, document) = try makeWindow(panes: 1)
      defer { document.close() }
      let original = try XCTUnwrap(controller.editorSurface as? Surface)
      let state = backend.persistenceState
      var result: Result<String?, Error>?
      controller.perform(documentHostRequests: [EVDocumentHostRequest(
        kind: .split, documentID: state.documentID, documentRevision: state.documentRevision,
        initialHeightRows: vertical ? 24 : 4, verticalSplit: vertical)
      ]) { result = $0 }
      _ = try XCTUnwrap(result).get()
      let added = try XCTUnwrap(controller.editorSurface as? Surface)
      XCTAssertFalse(added === original)
      XCTAssertTrue(added === backend.surfaces.last)
      func frame(_ surface: Surface) -> NSRect {
        surface.viewController.view.convert(surface.viewController.view.bounds, to: nil)
      }
      if vertical {
        XCTAssertLessThan(frame(added).minX, frame(original).minX)
        XCTAssertEqual(frame(added).width, 24 * 8, accuracy: 0.001)
      } else {
        XCTAssertGreaterThan(frame(added).minY, frame(original).minY)
        XCTAssertEqual(frame(added).height, 4 * 20, accuracy: 0.001)
      }
      XCTAssertTrue(controller.window?.firstResponder === added.viewController.view)
      controller.perform(windowRequests: [.focusLastAccessed], from: added)
      XCTAssertTrue(controller.editorSurface as? Surface === original)
    }
  }

  func testFocusCommandsWalkClampAndWrapThroughStackedPanes() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    let send = { (request: EVWindowRequest) in
      controller.perform(windowRequests: [request], from: controller.editorSurface)
    }
    XCTAssertEqual(focusedIndex(controller, backend), 0)
    send(.focusDown(count: 1))
    XCTAssertEqual(focusedIndex(controller, backend), 1)
    send(.focusDown(count: 5))
    XCTAssertEqual(focusedIndex(controller, backend), 2, "focus stops at the bottom")
    send(.focusUp(count: 1))
    XCTAssertEqual(focusedIndex(controller, backend), 1)
    send(.focusUp(count: 9))
    XCTAssertEqual(focusedIndex(controller, backend), 0, "focus stops at the top")
    send(.focusNext(index: nil))
    XCTAssertEqual(focusedIndex(controller, backend), 1)
    send(.focusNext(index: nil))
    send(.focusNext(index: nil))
    XCTAssertEqual(focusedIndex(controller, backend), 0, "next wraps to the top")
    send(.focusPrevious(index: nil))
    XCTAssertEqual(focusedIndex(controller, backend), 2, "previous wraps to the bottom")
    send(.focusNext(index: 2))
    XCTAssertEqual(focusedIndex(controller, backend), 1, "an index is one-based")
    send(.focusBottom)
    XCTAssertEqual(focusedIndex(controller, backend), 2)
    send(.focusTop)
    XCTAssertEqual(focusedIndex(controller, backend), 0)
  }

  func testLastAccessedAlternatesBetweenTwoPanes() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    let send = { (request: EVWindowRequest) in
      controller.perform(windowRequests: [request], from: controller.editorSurface)
    }
    send(.focusBottom)
    XCTAssertEqual(focusedIndex(controller, backend), 2)
    send(.focusLastAccessed)
    XCTAssertEqual(focusedIndex(controller, backend), 0)
    send(.focusLastAccessed)
    XCTAssertEqual(focusedIndex(controller, backend), 2, "it alternates")
  }

  func testRotateMovesEveryPaneAndKeepsTheSameOneFocused() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    let send = { (request: EVWindowRequest) in
      controller.perform(windowRequests: [request], from: controller.editorSurface)
    }
    let original = stackedSurfaces(controller, backend)
    send(.focusTop)
    send(.rotateDown(count: 1))
    var order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[0] === original[2], "the bottom pane becomes the top")
    XCTAssertTrue(order[1] === original[0])
    XCTAssertTrue(order[2] === original[1])
    XCTAssertEqual(focusedIndex(controller, backend), 1, "focus followed its pane")
    send(.rotateUp(count: 1))
    order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[0] === original[0], "rotating back restores the order")
    XCTAssertTrue(order[2] === original[2])
    XCTAssertEqual(focusedIndex(controller, backend), 0)
    send(.rotateDown(count: 3))
    order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[0] === original[0], "a full turn is the identity")
  }

  func testExchangeAndMoveReorderPanesAndCarryFocus() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    let send = { (request: EVWindowRequest) in
      controller.perform(windowRequests: [request], from: controller.editorSurface)
    }
    let original = stackedSurfaces(controller, backend)
    send(.focusTop)
    send(.exchange(index: nil))
    var order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[0] === original[1])
    XCTAssertTrue(order[1] === original[0])
    XCTAssertEqual(focusedIndex(controller, backend), 1, "focus follows the exchanged pane")
    send(.focusBottom)
    send(.exchange(index: nil))
    order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[2] === original[0], "the last pane exchanges with the previous one")
    XCTAssertEqual(focusedIndex(controller, backend), 1)

    send(.moveToTop)
    order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[0] === original[2])
    XCTAssertEqual(focusedIndex(controller, backend), 0)
    send(.moveToBottom)
    order = stackedSurfaces(controller, backend)
    XCTAssertTrue(order[2] === original[2])
    XCTAssertEqual(focusedIndex(controller, backend), 2)
  }

  func testHeightCommandsMoveWholeRowsAndEqualizeRestoresTheSplit() throws {
    let (controller, backend, document) = try makeWindow(panes: 2)
    defer { document.close() }
    let send = { (request: EVWindowRequest) in
      controller.perform(windowRequests: [request], from: controller.editorSurface)
    }
    func heights() -> [CGFloat] {
      stackedSurfaces(controller, backend).map {
        $0.viewController.view.convert($0.viewController.view.bounds, to: nil).height
      }
    }
    send(.focusTop)
    let start = heights()
    XCTAssertEqual(start[0], start[1], accuracy: 2)
    send(.grow(rows: 2))
    let grown = heights()
    XCTAssertEqual(grown[0] - start[0], 40, accuracy: 2, "two twenty-point rows")
    XCTAssertEqual(start[1] - grown[1], 40, accuracy: 2, "taken from the neighbour")
    send(.shrink(rows: 2))
    let restored = heights()
    XCTAssertEqual(restored[0], start[0], accuracy: 2)
    send(.grow(rows: 10_000))
    let clamped = heights()
    XCTAssertEqual(clamped[1], 0, accuracy: 1, "only its status bar remains")
    XCTAssertLessThan(clamped[1], start[1])
    send(.equalizeHeights)
    let equal = heights()
    XCTAssertEqual(equal[0], equal[1], accuracy: 2)
  }

  func testExtremeCountsAllowEditorsToCollapseWithoutOverlappingBars() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    let order = stackedSurfaces(controller, backend)
    order[1].rowHeight = 130
    order[2].rowHeight = 170
    controller.perform(windowRequests: [.focusTop, .setHeight(rows: Int.max)], from: controller.editorSurface)
    XCTAssertEqual(order[1].viewController.view.bounds.height, 0, accuracy: 1)
    XCTAssertEqual(order[2].viewController.view.bounds.height, 0, accuracy: 1)
    controller.perform(windowRequests: [.shrink(rows: Int.max)], from: controller.editorSurface)
    XCTAssertEqual(order[0].viewController.view.bounds.height, 0, accuracy: 1)
    controller.perform(windowRequests: [.grow(rows: Int.max)], from: controller.editorSurface)
    XCTAssertEqual(order[1].viewController.view.bounds.height, 0, accuracy: 1)
    controller.perform(windowRequests: [.focusDown(count: Int.max)], from: controller.editorSurface)
    XCTAssertEqual(focusedIndex(controller, backend), 2)
  }

  private func stack(panes count: Int, height: CGFloat = 600) throws -> EVPaneContainer {
    let container = EVPaneContainer(first: EVDocumentContentViewController(editorSurface: Surface()))
    container.loadViewIfNeeded()
    container.view.frame = NSRect(x: 0, y: 0, width: 600, height: height)
    for _ in 1..<count {
      try container.insert(EVDocumentContentViewController(editorSurface: Surface()))
      container.perform(.equalizeHeights)
    }
    return container
  }

  private func barTops(_ container: EVPaneContainer) -> [CGFloat] {
    container.panes.map { $0.view.frame.maxY - $0.statusBarHeight }
  }

  private func checkBars(_ container: EVPaneContainer, file: StaticString = #filePath, line: UInt = #line) {
    let tops = barTops(container)
    for index in 0..<tops.count - 1 {
      XCTAssertLessThanOrEqual(tops[index] + container.panes[index].statusBarHeight, tops[index + 1] + 0.001, file: file, line: line)
    }
    XCTAssertEqual(tops.last! + container.panes.last!.statusBarHeight, container.view.bounds.height, accuracy: 0.001, file: file, line: line)
  }

  func testDragCollectsBarsAtBothEdgesAndReversesImmediately() throws {
    let container = try stack(panes: 5)
    let grabbed = container.panes[2]
    let initial = barTops(container)
    container.dragStatusBar(of: grabbed, by: 10_000)
    let bottom = barTops(container)
    XCTAssertEqual(bottom[0], initial[0], accuracy: 0.001)
    XCTAssertEqual(bottom[1], initial[1], accuracy: 0.001)
    XCTAssertEqual(bottom[2] + grabbed.statusBarHeight, bottom[3], accuracy: 0.001)
    XCTAssertEqual(bottom[3] + container.panes[3].statusBarHeight, bottom[4], accuracy: 0.001)
    container.dragStatusBar(of: grabbed, by: 200)
    XCTAssertEqual(barTops(container), bottom)
    container.dragStatusBar(of: grabbed, by: -12)
    let reversed = barTops(container)
    XCTAssertEqual(reversed[2], bottom[2] - 12, accuracy: 0.001)
    XCTAssertEqual(reversed[3], bottom[3], accuracy: 0.001)
    container.dragStatusBar(of: grabbed, by: -10_000)
    let top = barTops(container)
    XCTAssertEqual(top[0], 0, accuracy: 0.001)
    XCTAssertEqual(top[0] + container.panes[0].statusBarHeight, top[1], accuracy: 0.001)
    XCTAssertEqual(top[1] + container.panes[1].statusBarHeight, top[2], accuracy: 0.001)
    XCTAssertEqual(top[3], bottom[3], accuracy: 0.001)
    container.dragStatusBar(of: grabbed, by: -200)
    container.dragStatusBar(of: grabbed, by: 12)
    let downAgain = barTops(container)
    XCTAssertEqual(downAgain[2], top[2] + 12, accuracy: 0.001)
    XCTAssertEqual(Array(downAgain.prefix(2)), Array(top.prefix(2)))
    checkBars(container)
  }

  func testBottomBarIsFixedAndSplitAdmissionUsesOnlyTheCurrentEditorGap() throws {
    let container = try stack(panes: 3)
    XCTAssertNil(container.panes.last!.statusBar.dragDidMove)
    let start = barTops(container)
    container.dragStatusBar(of: container.panes.last!, by: -300)
    XCTAssertEqual(barTops(container), start)
    container.focusPane(at: 0)
    container.perform(.setHeight(rows: 1))
    let panes = container.panes
    XCTAssertThrowsError(try container.insert(EVDocumentContentViewController(editorSurface: Surface()))) { error in
      XCTAssertEqual(error as? EVDocumentHostError, .noRoomToSplit)
    }
    XCTAssertEqual(container.panes.count, panes.count)
    XCTAssertTrue(container.activePane === panes[0])
    container.perform(.grow(rows: 1))
    XCTAssertNoThrow(try container.requireSplitRoom())
    checkBars(container)
  }

  func testExactlyOneBarOfRoomAllowsSplitAndTouchingBars() throws {
    let bar = EVStatusBarView.preferredHeight
    let bottomBar = EVStatusBarView(frame: .zero)
    bottomBar.hidesBottomBorder = true
    // The window edge clips the bottom border, which may be nonzero in a theme.
    let container = try stack(panes: 1, height: bar + bottomBar.visiblePreferredHeight)
    try container.insert(EVDocumentContentViewController(editorSurface: Surface()))
    XCTAssertEqual(container.panes.count, 2)
    XCTAssertEqual(container.panes[0].editorSurface.viewController.view.bounds.height, 0, accuracy: 0.001)
    XCTAssertEqual(container.panes[1].editorSurface.viewController.view.bounds.height, 0, accuracy: 0.001)
    checkBars(container)
  }

  func testWindowResizeKeepsCollapsedGapsAndFullBars() throws {
    let container = try stack(panes: 5)
    container.dragStatusBar(of: container.panes[0], by: 10_000)
    container.view.frame.size.height = 350
    checkBars(container)
    for pane in container.panes.dropFirst() {
      XCTAssertEqual(pane.editorSurface.viewController.view.bounds.height, 0, accuracy: 0.001, "pane: \(pane.view.frame), editor: \(pane.editorSurface.viewController.view.frame), bounds: \(pane.editorSurface.viewController.view.bounds)")
    }
  }

  func testReorderingMovesTheFixedBarRoleAndDraggingPreservesFocus() throws {
    let container = try stack(panes: 4)
    let oldBottom = container.panes.last!
    container.focusPane(at: 1)
    let focused = container.activePane
    container.dragStatusBar(of: container.panes[0], by: 15)
    XCTAssertTrue(container.activePane === focused)
    container.perform(.rotateDown(count: 1))
    XCTAssertTrue(container.panes[0] === oldBottom)
    XCTAssertNotNil(oldBottom.statusBar.dragDidMove)
    XCTAssertNil(container.panes.last!.statusBar.dragDidMove)
    XCTAssertTrue(container.activePane === focused)
    container.remove(container.panes.last!)
    XCTAssertNil(container.panes.last!.statusBar.dragDidMove)
    checkBars(container)
  }

  private func prepareForPointerEvents(in window: NSWindow) {
    let application = NSApplication.shared
    // Ordering a window schedules AppKit setup on the run loop. Finish that
    // setup before the first synthetic gesture, just as a displayed window
    // would before accepting real user input.
    window.contentView?.layoutSubtreeIfNeeded()
    window.displayIfNeeded()
    application.updateWindows()
    window.makeKeyAndOrderFront(nil)
    // AppKit ordering/activation events live in the application queue, not
    // just the run loop. Drain them before posting the synthetic mouse down.
    let deadline = Date(timeIntervalSinceNow: 0.05)
    for _ in 0..<128 {
      guard Date() < deadline,
        let event = application.nextEvent(matching: .any, until: deadline,
          inMode: .default, dequeue: true) else { break }
      application.sendEvent(event)
    }
    window.contentView?.layoutSubtreeIfNeeded()
    (window as? PointerTestWindow)?.makeKeyForTesting()
    // Reassert the declared fixture after pending initial-ordering events, as
    // makeWindow does for document windows after showWindow.
    window.setContentSize(NSSize(width: 600, height: 600))
    window.contentView?.layoutSubtreeIfNeeded()
  }

  /// Drive native window hit-testing, recognizers and deferred button clicks.
  /// Queue trailing events before mouseDown because controls can enter tracking.
  private func dispatchPointerGesture(window: NSWindow, points: [NSPoint]) throws {
    let events = try points.enumerated().map { index, point in
      try XCTUnwrap(NSEvent.mouseEvent(
        with: index == 0 ? .leftMouseDown : index == points.count - 1 ? .leftMouseUp : .leftMouseDragged,
        location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime + Double(index) * 0.02,
        windowNumber: window.windowNumber, context: nil, eventNumber: index + 1, clickCount: 1, pressure: index == points.count - 1 ? 0 : 1))
    }
    // An inactive test host can discard a newly posted initial down before it
    // reaches native controls. Deliver it directly, with the remaining gesture
    // ahead of old window events for native tracking loops and the bounded pump.
    for event in events.dropFirst().reversed() { NSApplication.shared.postEvent(event, atStart: true) }
    window.sendEvent(events[0])
    // Wait for a quiet queue, rather than cutting off a gesture when native
    // dispatch/layout takes longer on a loaded machine. The event count bounds
    // the pump even if an unrelated producer keeps posting events.
    for _ in 0..<256 {
      guard let event = NSApplication.shared.nextEvent(matching: .any,
        until: Date(timeIntervalSinceNow: 0.1), inMode: .default, dequeue: true) else { break }
      // Exercise this window's native dispatch even when the unattended test
      // host cannot activate over another app. Preserve other events' targets.
      if event.windowNumber == window.windowNumber,
        [.leftMouseDown, .leftMouseDragged, .leftMouseUp].contains(event.type) {
        window.sendEvent(event)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
      } else {
        NSApplication.shared.sendEvent(event)
      }
    }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
  }

  private func pointerGesture(window: NSWindow, points: [NSPoint]) async throws {
    try dispatchPointerGesture(window: window, points: points)
    // AppKit releases deferred clicks on the main queue. A nested run loop in
    // a synchronous main-actor test cannot drain that queue; yield the actor as
    // the native measurement-click fixtures do.
    await withCheckedContinuation { continuation in
      DispatchQueue.main.async { continuation.resume() }
    }
  }

  func testVerticalSplitterTranslucentCornersArePaintedOnce() throws {
    let splitter = EVVerticalSplitter(frame: NSRect(x: 0, y: 0, width: 5, height: 20))
    var geometry = ViemPaneFrame()
    geometry.y = 10; geometry.height = 20
    splitter.updateBorderVisibility(frame: geometry, containerHeight: 100)
    for width in [CGFloat(1), 4] {
      let border = EVChromeBorder(width: width, color: .black.withAlphaComponent(0.5))
      let style = EVChromeStyle(topBorder: border, rightBorder: border, bottomBorder: border, leftBorder: border)
      let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil,
        pixelsWide: 5, pixelsHigh: 20, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
      let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
      NSGraphicsContext.saveGraphicsState()
      NSGraphicsContext.current = graphics
      graphics.cgContext.clear(splitter.bounds)
      splitter.drawBorders(style)
      NSGraphicsContext.restoreGraphicsState()
      for x in [0, 2, 4] {
        for y in [0, 19] {
          let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y))
          XCTAssertEqual(color.alphaComponent, 0.5, accuracy: 0.01, "width \(width), pixel \(x),\(y)")
        }
      }
    }
  }

  func testVerticalSplitterUnderStatusBarOmitsTopCapWhenUpperPaneCollapses() throws {
    let container = try stack(panes: 2)
    let upper = container.panes[0], lower = container.panes[1]
    let right = EVDocumentContentViewController(editorSurface: Surface())
    try container.insert(right, splitting: lower, vertical: true)
    let splitter = try XCTUnwrap(container.view.subviews.compactMap { $0 as? EVVerticalSplitter }.first)
    XCTAssertGreaterThan(splitter.frame.minY, 0)
    XCTAssertTrue(splitter.hidesTopBorder, "The horizontal status bar above owns the T junction")
    XCTAssertTrue(splitter.hidesBottomBorder, "The bottom still touches the window edge")
    container.dragStatusBar(of: upper, by: -10_000)
    XCTAssertEqual(upper.editorSurface.viewController.view.bounds.height, 0, accuracy: 0.001)
    XCTAssertEqual(splitter.frame.minY, upper.statusBarHeight, accuracy: 0.001)
    XCTAssertTrue(splitter.hidesTopBorder, "A collapsed editor retains its real status-bar coverage")

    // The portable suppression bit removes only the horizontal cap. Vertical
    // side borders still reach the junction and the opposite cap is unchanged.
    let sample = EVVerticalSplitter(frame: NSRect(x: 0, y: 0, width: 5, height: 20))
    var frame = ViemPaneFrame()
    frame.y = 10; frame.height = 20
    frame.flags = UInt32(VIEM_PANE_FRAME_SUPPRESS_SPLITTER_TOP_BORDER)
    sample.updateBorderVisibility(frame: frame, containerHeight: 100)
    let border = EVChromeBorder(width: 1, color: .black.withAlphaComponent(0.5))
    let style = EVChromeStyle(topBorder: border, rightBorder: border, bottomBorder: border, leftBorder: border)
    let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil,
      pixelsWide: 5, pixelsHigh: 20, bitsPerSample: 8, samplesPerPixel: 4,
      hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
    let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = graphics
    graphics.cgContext.clear(sample.bounds)
    sample.drawBorders(style)
    NSGraphicsContext.restoreGraphicsState()
    XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 2, y: 0)).alphaComponent, 0, accuracy: 0.01)
    XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 0, y: 0)).alphaComponent, 0.5, accuracy: 0.01)
    XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 4, y: 0)).alphaComponent, 0.5, accuracy: 0.01)
    XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 2, y: 19)).alphaComponent, 0.5, accuracy: 0.01)
  }

  func testNestedVerticalSplittersClipBordersOnlyAtOuterWindowEdges() throws {
    let container = try stack(panes: 3)
    let rows = container.panes
    var rightPanes: [EVDocumentContentViewController] = []
    for row in rows {
      let right = EVDocumentContentViewController(editorSurface: Surface())
      try container.insert(right, splitting: row, vertical: true)
      rightPanes.append(right)
    }
    container.layoutPanes()
    let splitters = container.view.subviews.compactMap { $0 as? EVVerticalSplitter }
      .sorted { $0.frame.minY < $1.frame.minY }
    XCTAssertEqual(splitters.count, 3)
    XCTAssertEqual(splitters.map(\.hidesTopBorder), [true, false, false])
    XCTAssertEqual(splitters.map(\.hidesBottomBorder), [false, false, true])
    XCTAssertTrue(splitters.allSatisfy { $0.frame.width == 5 })
    // A surviving nested divider reaches both outer edges after the other
    // rows close; clipping must be recomputed from the new portable frame.
    for pane in container.panes where pane !== rows[0] && pane !== rightPanes[0] {
      container.remove(pane)
    }
    let survivor = try XCTUnwrap(container.view.subviews.compactMap { $0 as? EVVerticalSplitter }.first)
    XCTAssertTrue(survivor === splitters[0])
    XCTAssertTrue(survivor.hidesTopBorder)
    XCTAssertTrue(survivor.hidesBottomBorder)
    XCTAssertEqual(survivor.frame.height, container.view.bounds.height)
  }

  func testVerticalNativeSplittersPushReverseAndStatusClicksFocusWithoutDragFocus() async throws {
    let container = try stack(panes: 1)
    let first = container.panes[0]
    let second = EVDocumentContentViewController(editorSurface: Surface())
    try container.insert(second, splitting: first, vertical: true)
    let third = EVDocumentContentViewController(editorSurface: Surface())
    container.focusPane(at: 1)
    container.perform(.setWidth(columns: nil))
    try container.insert(third, splitting: second, vertical: true)
    container.perform(.equalizeHeights)
    let window = PointerTestWindow(contentRect: container.view.bounds, styleMask: [.titled,.resizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.tabbingMode = .disallowed
    container.view.translatesAutoresizingMaskIntoConstraints = false
    window.contentView = container.view
    window.setContentSize(NSSize(width: 600,height: 600))
    window.makeKeyAndOrderFront(nil)
    defer { window.close() }
    prepareForPointerEvents(in: window)
    XCTAssertEqual(container.view.bounds.size, NSSize(width: 600, height: 600),
      "window frame: \(window.frame), content frame: \(container.view.frame), root bounds: \(container.view.bounds), screen: \(String(describing: window.screen))")
    XCTAssertTrue(container.panes.allSatisfy { $0.view.frame.width >= 100 })
    XCTAssertTrue(container.panes.allSatisfy { $0.statusBar.dragDidMove == nil })
    let divider = try XCTUnwrap(container.view.subviews.compactMap { $0 as? EVVerticalSplitter }.sorted { $0.frame.minX < $1.frame.minX }.first)
    XCTAssertEqual(divider.frame.width,5)
    let grab = divider.convert(NSPoint(x:2.5,y:divider.bounds.midY),to:nil)
    var dragCursorTransitions: [Bool] = []
    let move = divider.dragDidMove
    divider.dragDidMove = { delta in
      dragCursorTransitions.append(NSCursor.current == NSCursor.resizeLeftRight)
      move?(delta)
    }
    container.focusPane(at: 2)
    try await pointerGesture(window:window,points:[grab,NSPoint(x:grab.x+800,y:grab.y),NSPoint(x:grab.x+850,y:grab.y),NSPoint(x:grab.x+838,y:grab.y),NSPoint(x:grab.x+838,y:grab.y)])
    // Assert the transition into dragging. Later cursor-rectangle updates may
    // reflect the real pointer's location while synthetic moves resize views.
    XCTAssertEqual(dragCursorTransitions.first, true,
      "The first native drag must install the resize cursor; observed: \(dragCursorTransitions)")
    XCTAssertTrue(container.activePane === third,"a splitter drag preserves focus")
    XCTAssertEqual(first.view.frame.width,378,accuracy:1)
    XCTAssertEqual(second.view.frame.width,112,accuracy:1)
    XCTAssertEqual(third.view.frame.width,100,accuracy:1)
    container.focusPane(at:0)
    container.perform(.setWidth(columns:1))
    XCTAssertEqual(first.view.frame.width,100,accuracy:0.001)
    XCTAssertThrowsError(try container.requireSplitRoom(in:first,vertical:true))
    container.view.layoutSubtreeIfNeeded()
    let statusPoint = second.statusBar.convert(NSPoint(x:second.statusBar.bounds.midX,y:second.statusBar.bounds.midY),to:nil)
    try await pointerGesture(window:window,points:[statusPoint,statusPoint])
    XCTAssertTrue(container.activePane === second,"a plain status background click focuses its buffer")
    let below = EVDocumentContentViewController(editorSurface:Surface())
    try container.insert(below,splitting:second)
    container.focusPane(at:container.panes.firstIndex(where:{$0 === third})!)
    let point = second.statusBar.convert(NSPoint(x:second.statusBar.bounds.midX,y:second.statusBar.bounds.midY),to:nil)
    var statusCursorTransitions: [Bool] = []
    let statusMove = second.statusBar.dragDidMove
    second.statusBar.dragDidMove = { delta in
      statusCursorTransitions.append(NSCursor.current == NSCursor.resizeUpDown)
      statusMove?(delta)
    }
    try await pointerGesture(window:window,points:[point,NSPoint(x:point.x,y:point.y-30),NSPoint(x:point.x,y:point.y-30)])
    XCTAssertEqual(statusCursorTransitions.first, true,
      "The first native drag must install the resize cursor; observed: \(statusCursorTransitions)")
    XCTAssertTrue(container.activePane === third,"a status-bar drag preserves focus")
    XCTAssertNil(below.statusBar.dragDidMove)
  }

  func testCloseOthersLeavesOnlyTheFocusedPane() throws {
    let (controller, backend, document) = try makeWindow(panes: 3)
    defer { document.close() }
    controller.perform(windowRequests: [.focusDown(count: 1)], from: controller.editorSurface)
    let kept = controller.editorSurface
    controller.perform(windowRequests: [.closeOthers], from: controller.editorSurface)
    XCTAssertEqual(controller.paneCount, 1)
    XCTAssertTrue(controller.editorSurface === kept)
    _ = backend
  }

  func testRequestsFromAForeignSurfaceAreIgnored() throws {
    let (controller, backend, document) = try makeWindow(panes: 2)
    defer { document.close() }
    let intruder = Surface()
    let before = focusedIndex(controller, backend)
    controller.perform(windowRequests: [.focusBottom], from: intruder)
    XCTAssertEqual(focusedIndex(controller, backend), before)
  }
}
