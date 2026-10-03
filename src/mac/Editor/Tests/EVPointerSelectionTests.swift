import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVPointerSelectionTests: XCTestCase {
  func testSingleClickPreservesModeAndFollowingCommandOrTextInput() throws {
    let source = "before chosen after"
    for (command, mode) in [
      ("", UInt32(VIEM_MODE_NORMAL)),
      ("i", UInt32(VIEM_MODE_INSERT)),
      ("R", UInt32(VIEM_MODE_REPLACE)),
    ] {
      let (backend, surface, window) = try makeSurface(source, width: 600)
      defer { withExtendedLifetime(window) {} }
      let session = try XCTUnwrap(surface.session)
      if !command.isEmpty {
        surface.performInput { _ = try session.sendText(command) }
      }
      let point = try click(surface, at: 7, count: 1)
      surface.editorView.mouseUp(with: try pointerEvent(surface, type: .leftMouseUp, at: point))
      XCTAssertEqual(surface.viewPresentation.mode, mode)
      XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)
      XCTAssertTrue(surface.selectedUTF8Ranges().isEmpty)
      XCTAssertEqual(
        try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
      XCTAssertFalse(backend.persistenceState.isDirty)

      if mode == UInt32(VIEM_MODE_NORMAL) {
        let event = try XCTUnwrap(NSEvent.keyEvent(
          with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
          windowNumber: window.windowNumber, context: nil, characters: "l",
          charactersIgnoringModifiers: "l", isARepeat: false, keyCode: 37))
        surface.editorView.keyDown(with: event)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 8)
        XCTAssertEqual(try backend.formattedText(), source)
      } else {
        surface.editorView.insertText(
          "X", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 8)
        XCTAssertEqual(
          try backend.formattedText(),
          mode == UInt32(VIEM_MODE_INSERT) ? "before Xchosen after" : "before Xhosen after")
      }
      XCTAssertEqual(surface.viewPresentation.mode, mode)
    }
  }

  func testOffscreenDragKeepsItsAnchorAndVisibleSelectionAcrossLongDocuments() throws {
    var checkout = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { checkout.deleteLastPathComponent() }
    let agents = try String(contentsOf: checkout.appendingPathComponent("AGENTS.md"), encoding: .utf8)
    let fixtures = [
      ("AGENTS.md", agents, EVDocument.markdownType),
      ("20,000 lines", String(repeating: "Words continue across a long document with wrapped lines of text.\n", count: 20_000), EVDocument.plainTextType),
    ]
    for (label, source, format) in fixtures {
      let (backend, surface, window) = try makeSurface(source, width: 480, format: format)
      defer { withExtendedLifetime(window) {} }
      let view = surface.editorView
      try click(surface, at: 4, count: 1)
      let anchor = surface.viewPresentation.cursor_utf8_offset
      let viewport = view.bounds
      let outside = NSPoint(x: viewport.midX, y: viewport.maxY + 2000)
      let drag = try pointerEvent(surface, type: .leftMouseDragged, at: outside)
      let release = try pointerEvent(surface, type: .leftMouseUp, at: outside)
      defer { view.mouseUp(with: release) }
      view.mouseDragged(with: drag)
      let initialTop = surface.viewportState.top

      func assertSelection(_ step: String) throws {
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        XCTAssertEqual(surface.viewPresentation.visual_anchor_utf8_offset, anchor,
                       "\(label), step \(step): the drag must retain its original anchor")
        let selection = try XCTUnwrap(surface.visualSelection,
          "\(label), step \(step): selection disappeared after scrolling to \(surface.viewportState.top)")
        XCTAssertEqual(selection.segments.first?.text_start, anchor)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertTrue(view.selectionRectsForDrawing(in: snapshot).contains { $0.intersects(view.bounds) },
                      "\(label), step \(step): selected visible rows lost their highlight")
      }

      for step in 0..<160 {
        XCTAssertTrue(view.performDragAutoscrollStep(), "\(label), step \(step)")
        try assertSelection("down \(step)")
      }

      XCTAssertGreaterThan(surface.viewportState.top, initialTop + Float(viewport.height * 3))
      let downwardEnd = surface.viewPresentation.cursor_utf8_offset
      let above = NSPoint(x: viewport.midX, y: viewport.minY - 2000)
      view.mouseDragged(with: try pointerEvent(surface, type: .leftMouseDragged, at: above))
      for step in 0..<40 {
        XCTAssertTrue(view.performDragAutoscrollStep(), "\(label), reverse step \(step)")
        try assertSelection("up \(step)")
      }
      XCTAssertLessThan(surface.viewPresentation.cursor_utf8_offset, downwardEnd)
      let selectionBeforeRelease = view.selectedRange()
      XCTAssertNotEqual(selectionBeforeRelease.location, NSNotFound)
      XCTAssertGreaterThan(selectionBeforeRelease.length, 0)
      view.mouseUp(with: release)
      XCTAssertFalse(view.isDragAutoscrollActive)
      XCTAssertEqual(view.selectedRange(), selectionBeforeRelease)
      XCTAssertEqual(try backend.serializedSource(typeName: format), Data(source.utf8))
      XCTAssertFalse(backend.persistenceState.isDirty)
    }
  }

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

  func testDoubleClickDragKeepsWholeWordsThroughDirectionChanges() throws {
    let source = "first naïve e\u{301}lan final"
    for command in ["", "i", "R"] {
      let (backend, surface, window) = try makeSurface(source, width: 600)
      defer { withExtendedLifetime(window) {} }
      let session = try XCTUnwrap(surface.session)
      if !command.isEmpty { surface.performInput { _ = try session.sendText(command) } }
      try click(surface, at: 8, count: 2)
      for (offset, selected) in [(UInt64(16), "naïve e\u{301}lan"), (2, "first naïve"), (10, "naïve")] {
        let local = try point(surface, at: offset)
        surface.editorView.mouseDragged(with: try pointerEvent(surface, type: .leftMouseDragged, at: local))
        XCTAssertEqual(surface.editorView.selectedRange(), (source as NSString).range(of: selected))
        XCTAssertFalse(surface.editorView.selectionRectsForDrawing(in: try XCTUnwrap(surface.layoutSnapshot)).isEmpty)
      }
      let local = try point(surface, at: 10)
      surface.editorView.mouseUp(with: try pointerEvent(surface, type: .leftMouseUp, at: local))
      XCTAssertEqual(surface.editorView.selectedRange(), (source as NSString).range(of: "naïve"))
      surface.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertEqual(try backend.formattedText(), "first X e\u{301}lan final")
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.formattedText(), source)
    }
  }

  func testWordDragRetainsGranularityDuringAutoscrollThenNewClickResetsIt() throws {
    let source = String(repeating: "alpha bravo charlie delta\n", count: 2_000)
    let (_, surface, window) = try makeSurface(source, width: 400)
    defer { withExtendedLifetime(window) {} }
    let view = surface.editorView
    try click(surface, at: 14, count: 2)
    let outside = NSPoint(x: 90, y: view.bounds.maxY + 500)
    view.mouseDragged(with: try pointerEvent(surface, type: .leftMouseDragged, at: outside))
    for _ in 0..<5 {
      XCTAssertTrue(view.performDragAutoscrollStep())
      let range = view.selectedRange()
      XCTAssertEqual(range.location, 12)
      let end = NSMaxRange(range)
      let text = source as NSString
      XCTAssertFalse(CharacterSet.letters.contains(UnicodeScalar(text.character(at: end))!),
        "The active endpoint must finish a whole word after autoscroll")
    }
    view.mouseUp(with: try pointerEvent(surface, type: .leftMouseUp, at: outside))
    XCTAssertFalse(view.isDragAutoscrollActive)
    surface.goToLine(1)
    try click(surface, at: 1, count: 1)
    view.mouseDragged(with: try pointerEvent(surface, type: .leftMouseDragged, at: point(surface, at: 3)))
    XCTAssertEqual(view.selectedRange(), NSRange(location: 1, length: 2))
    view.mouseUp(with: try pointerEvent(surface, type: .leftMouseUp, at: point(surface, at: 3)))
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

  func testDoubleClickFromInsertOrReplaceSelectsTheHitWordBeforeTyping() throws {
    let source = "before chosen after"
    for (command, mode) in [("i", UInt32(VIEM_MODE_INSERT)), ("R", UInt32(VIEM_MODE_REPLACE))] {
      let (backend, surface, window) = try makeSurface(source, width: 600)
      defer { withExtendedLifetime(window) {} }
      let session = try XCTUnwrap(surface.session)
      surface.performInput { _ = try session.sendText(command) }
      try click(surface, at: 7, count: 1)
      XCTAssertEqual(surface.viewPresentation.mode, mode)
      XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)

      // The first glyph's leading edge hit-tests to the word's start. Leaving
      // Insert after that hit would incorrectly move viw onto the prior space.
      try click(surface, at: 7, count: 2)
      XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
      XCTAssertEqual(surface.editorView.selectedRange(), (source as NSString).range(of: "chosen"))
      XCTAssertEqual(try backend.formattedText(), source)
      surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_RIGHT)) }
      XCTAssertEqual(surface.viewPresentation.mode, mode)
      try click(surface, at: 7, count: 2)
      surface.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertEqual(try backend.formattedText(), "before X after")
      surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.formattedText(), source)
    }
  }

  private func makeSurface(_ source: String, width: CGFloat, format: String? = nil) throws -> (
    EVCoreDocumentBackend, EVEditorSurfaceController, NSWindow
  ) {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: format ?? EVDocument.plainTextType)
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

  @discardableResult
  private func click(_ surface: EVEditorSurfaceController, at offset: UInt64, count: Int) throws -> NSPoint {
    let local = try point(surface, at: offset)
    let point = surface.editorView.convert(local, to: nil)
    let event = try XCTUnwrap(
      NSEvent.mouseEvent(
        with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0,
        windowNumber: surface.editorView.window?.windowNumber ?? 0, context: nil, eventNumber: 1,
        clickCount: count, pressure: 1))
    surface.editorView.mouseDown(with: event)
    return local
  }

  private func point(_ surface: EVEditorSurfaceController, at offset: UInt64) throws -> NSPoint {
    surface.refreshPresentation()
    let session = try XCTUnwrap(surface.session)
    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
    let geometry = try session.caretGeometry(
      offset: offset, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
    let rect = geometry.rect
    return surface.editorView.viewPoint(
      fromLayoutPoint: CGPoint(x: CGFloat(rect.x), y: CGFloat(rect.y + rect.height * 0.5)))
  }

  private func pointerEvent(_ surface: EVEditorSurfaceController, type: NSEvent.EventType,
                            at point: NSPoint) throws -> NSEvent {
    try XCTUnwrap(NSEvent.mouseEvent(with: type,
      location: surface.editorView.convert(point, to: nil), modifierFlags: [], timestamp: 0,
      windowNumber: surface.editorView.window?.windowNumber ?? 0, context: nil,
      eventNumber: 1, clickCount: 1, pressure: 1))
  }
}
