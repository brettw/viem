import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVPointerSelectionTests: XCTestCase {
  func testDoubleClickUsesPortableWordBoundariesAndDoesNotEditSource() throws {
    let source = "before naïve e\u{301}lan after"
    let (backend, surface, window) = try makeSurface(source, width: 600)
    defer { withExtendedLifetime(window) {} }
    try click(surface, at: 8, count: 2)
    XCTAssertEqual(surface.editorView.selectedRange(), (source as NSString).range(of: "naïve"))
    let combiningStart = UInt64(source.utf8.count - "e\u{301}lan after".utf8.count)
    try click(surface, at: combiningStart, count: 2)
    XCTAssertEqual(
      surface.editorView.selectedRange(), (source as NSString).range(of: "e\u{301}lan"))
    XCTAssertEqual(
      try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
  }

  func testTripleClickUsesSelectedVisualOrPhysicalLinePolicy() throws {
    let line = "first words continue for several wrapped rows of text"
    let source = line + "\nlast"
    let (backend, surface, window) = try makeSurface(source, width: 185)
    defer { withExtendedLifetime(window) {} }
    let session = try XCTUnwrap(surface.session)
    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
    let first = try XCTUnwrap(snapshot.rows.first)
    XCTAssertLessThan(first.text_end, UInt64(line.utf8.count))
    try click(surface, at: first.text_start, count: 3)
    let visual = try session.listSelection()
    XCTAssertEqual(visual.text_start, first.text_start)
    XCTAssertEqual(visual.text_end, first.text_end)
    surface.performInput { try session.setLineMode(.physicalSource) }
    try click(surface, at: 2, count: 3)
    let physical = try session.listSelection()
    XCTAssertEqual(physical.text_start, 0)
    XCTAssertEqual(physical.text_end, UInt64(line.utf8.count + 1))
    XCTAssertEqual(
      try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
  }

  private func makeSurface(_ source: String, width: CGFloat) throws -> (
    EVCoreDocumentBackend, EVEditorSurfaceController, NSWindow
  ) {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: width, height: 400), styleMask: [.titled],
      backing: .buffered, defer: false)
    window.contentViewController = surface
    surface.view.frame = NSRect(x: 0, y: 0, width: width, height: 400)
    surface.viewDidLayout()
    surface.refreshPresentation()
    return (backend, surface, window)
  }

  private func click(_ surface: EVEditorSurfaceController, at offset: UInt64, count: Int) throws {
    surface.refreshPresentation()
    let session = try XCTUnwrap(surface.session)
    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
    let geometry = try session.caretGeometry(
      offset: offset, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
    let rect = geometry.rect
    let local = surface.editorView.viewPoint(
      fromLayoutPoint: CGPoint(x: CGFloat(rect.x), y: CGFloat(rect.y + rect.height * 0.5)))
    let point = surface.editorView.convert(local, to: nil)
    let event = try XCTUnwrap(
      NSEvent.mouseEvent(
        with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0,
        windowNumber: surface.editorView.window?.windowNumber ?? 0, context: nil, eventNumber: 1,
        clickCount: count, pressure: 1))
    surface.editorView.mouseDown(with: event)
  }
}
