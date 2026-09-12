import AppKit
import XCTest

@testable import ViemAppShell

/// `CTRL-W` window effects against real stacked panes: focus order, pane
/// reordering, and heights measured in visual rows.
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
    var visualRowHeight: CGFloat? { 20 }
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
    XCTAssertGreaterThan(clamped[1], 0, "a neighbour never vanishes")
    XCTAssertLessThan(clamped[1], start[1])
    send(.equalizeHeights)
    let equal = heights()
    XCTAssertEqual(equal[0], equal[1], accuracy: 2)
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
