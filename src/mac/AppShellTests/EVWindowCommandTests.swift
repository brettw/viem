import AppKit
import XCTest

@testable import ViemAppShell

/// `CTRL-W` window effects against real stacked panes: focus order, pane
/// reordering, and heights measured in default paragraph lines.
@MainActor
final class EVWindowCommandTests: XCTestCase {
  private final class EditorTestView: NSView {
    override var acceptsFirstResponder: Bool { true }
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
    var defaultLineHeight: CGFloat? { rowHeight }
    var statusClicks = 0
    func perform(statusOption _: EVStatusBarOption) { statusClicks += 1 }
    func perform(menuCommand _: EVMenuCommand, sender _: Any?) {}
    func presentation(for _: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
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
    let container = try stack(panes: 1, height: bar * 2)
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
      XCTAssertEqual(pane.editorSurface.viewController.view.bounds.height, 0, accuracy: 0.001)
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

  /// Drive the AppKit event path, including deferred button clicks. Queue the
  /// complete gesture before dispatch because controls may run a tracking loop.
  private func pointerGesture(window: NSWindow, points: [NSPoint]) throws {
    let events = try points.enumerated().map { index, point in
      try XCTUnwrap(NSEvent.mouseEvent(
        with: index == 0 ? .leftMouseDown : index == points.count - 1 ? .leftMouseUp : .leftMouseDragged,
        location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime + Double(index) * 0.02,
        windowNumber: window.windowNumber, context: nil, eventNumber: index + 1, clickCount: 1, pressure: index == points.count - 1 ? 0 : 1))
    }
    for event in events { NSApplication.shared.postEvent(event, atStart: false) }
    while let event = NSApplication.shared.nextEvent(matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp],
      until: Date(timeIntervalSinceNow: 0.1), inMode: .default, dequeue: true) {
      window.sendEvent(event)
    }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
  }

  func testEyeClicksAndDragsUseTheNativeGesturePath() throws {
    let container = try stack(panes: 3)
    let window = NSWindow(contentRect: container.view.bounds, styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    container.view.translatesAutoresizingMaskIntoConstraints = false
    window.contentView = container.view
    window.setContentSize(NSSize(width: 600, height: 600))
    window.makeKeyAndOrderFront(nil)
    defer { window.close() }
    container.view.layoutSubtreeIfNeeded()
    let first = container.panes[0]
    let surface = try XCTUnwrap(first.editorSurface as? Surface)
    let button = try XCTUnwrap(first.statusBar.subviews.compactMap { $0 as? NSButton }.first)
    let eye = button.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), to: nil)
    let start = barTops(container)
    XCTAssertGreaterThan(button.bounds.width, 0)
    XCTAssertEqual(container.view.bounds.height, 600, accuracy: 1)
    try pointerGesture(window: window, points: [eye, eye])
    XCTAssertEqual(surface.statusClicks, 1, "a plain eye click toggles line mode")
    try pointerGesture(window: window, points: [eye, NSPoint(x: eye.x, y: eye.y - 40), NSPoint(x: eye.x, y: eye.y - 40)])
    XCTAssertEqual(surface.statusClicks, 1, "dragging the eye must consume its click")
    XCTAssertEqual(barTops(container)[0] - start[0], 40, accuracy: 1)
    let movedEye = button.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), to: nil)
    try pointerGesture(window: window, points: [movedEye,
      NSPoint(x: movedEye.x, y: movedEye.y - 800),
      NSPoint(x: movedEye.x, y: movedEye.y - 850),
      NSPoint(x: movedEye.x, y: movedEye.y - 838),
      NSPoint(x: movedEye.x, y: movedEye.y - 838)])
    let maximum = container.view.bounds.height - container.panes.reduce(CGFloat(0)) { $0 + $1.statusBarHeight }
    XCTAssertEqual(barTops(container)[0], maximum - 12, accuracy: 1, "native dragging reverses immediately after overshooting the edge")
    XCTAssertEqual(surface.statusClicks, 1)
    let bottom = container.panes.last!
    let bottomSurface = try XCTUnwrap(bottom.editorSurface as? Surface)
    let bottomButton = try XCTUnwrap(bottom.statusBar.subviews.compactMap { $0 as? NSButton }.first)
    let bottomEye = bottomButton.convert(NSPoint(x: bottomButton.bounds.midX, y: bottomButton.bounds.midY), to: nil)
    let beforeBottom = barTops(container)
    try pointerGesture(window: window, points: [bottomEye, bottomEye])
    XCTAssertEqual(bottomSurface.statusClicks, 1)
    XCTAssertEqual(barTops(container), beforeBottom)
    checkBars(container)
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
