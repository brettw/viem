import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor
@testable import ViemCoreTextProvider

@MainActor
final class EVCaretHoverTests: XCTestCase {
    func testPreviewMatchesPlainClickAcrossModesUnicodeAndBlankSpaceWithoutChangingState() throws {
        let source = "Wi e\u{301} 👩‍💻 אב\n\nlast"
        for command in ["", "i", "R", "v"] {
            let (surface, window) = try makeSurface(source)
            defer { withExtendedLifetime(window) {} }
            let session = try XCTUnwrap(surface.session)
            for offset in [0, 3, 7, 19] as [UInt64] {
                for fraction: CGFloat in [0.1, 0.9] {
                    surface.performInput {
                        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                        if !command.isEmpty { _ = try session.sendText(command) }
                    }
                    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
                    guard let cluster = snapshot.clusters.first(where: { $0.text_start == offset }) else { continue }
                    let point = surface.editorView.viewPoint(fromLayoutPoint: CGPoint(
                        x: CGFloat(cluster.x) + CGFloat(cluster.advance) * fraction,
                        y: CGFloat(cluster.typographic_bounds.y + cluster.typographic_bounds.height / 2)))
                    try assertPreviewMatchesClick(surface, at: point)
                }
            }
            // Gaps before/after text, a blank line, and canvas below EOF all
            // resolve through the exact same normalization as a plain click.
            for rowIndex in [0, 1, 2] {
                surface.performInput {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                    if !command.isEmpty { _ = try session.sendText(command) }
                }
                let row = try XCTUnwrap(surface.layoutSnapshot).rows[rowIndex]
                for x: CGFloat in [1, 450] {
                    let point = surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: x, y: CGFloat(row.y + row.ascent / 2)))
                    try assertPreviewMatchesClick(surface, at: point)
                }
            }
            try assertPreviewMatchesClick(surface, at: NSPoint(x: 450, y: 270))
            XCTAssertEqual(try surface.backend.formattedText(), source)
            XCTAssertFalse(surface.backend.persistenceState.isDirty)
        }
    }

    func testTypingHidesUntilMovementExceedsThresholdAndPreferenceUpdatesLive() throws {
        let (surface, window) = try makeSurface("WWWWWWWWWWWWWW")
        defer { withExtendedLifetime(window) {} }
        let view = surface.editorView
        let session = try XCTUnwrap(surface.session)
        let point = try cellPoint(surface, offset: 8)
        try move(view, to: point)
        XCTAssertNotNil(view.caretHoverRect)
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "i", charactersIgnoringModifiers: "i", isARepeat: false, keyCode: 34))
        view.keyDown(with: event)
        XCTAssertNil(view.caretHoverRect)
        for dx: CGFloat in [1, 2, 3, 4, 2, 0] {
            try move(view, to: NSPoint(x: point.x + dx, y: point.y))
            XCTAssertNil(view.caretHoverRect, "jitter \(dx) must not re-enable the effect")
        }
        let moved = NSPoint(x: point.x + 5, y: point.y)
        try move(view, to: moved)
        XCTAssertNotNil(view.caretHoverRect)
        XCTAssertEqual(view.caretHoverMode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(view.caretHoverRect?.width, 2)
        view.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertNil(view.caretHoverRect)
        try move(view, to: moved)
        XCTAssertNil(view.caretHoverRect)
        try move(view, to: point)
        XCTAssertNotNil(view.caretHoverRect)
        view.editingPreferences.setCaretHoverEffect(false)
        XCTAssertNil(view.caretHoverRect)
        view.editingPreferences.setCaretHoverEffect(true)
        XCTAssertNotNil(view.caretHoverRect)
        view.setMarkedText("e", selectedRange: NSRange(location: 1, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertNil(view.caretHoverRect)
        view.unmarkText()
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        try move(view, to: NSPoint(x: point.x + 10, y: point.y))
        XCTAssertNotNil(view.caretHoverRect)
        window.setKeyWindowForTesting(false)
        XCTAssertNil(view.caretHoverRect)
    }

    func testHoverUsesExistingLargeDocumentGeometryAndRejectsStaleSnapshots() throws {
        let (surface, window) = try makeSurface(String(repeating: "alpha bravo charlie delta\n", count: 20_000))
        defer { withExtendedLifetime(window) {} }
        let session = try XCTUnwrap(surface.session)
        let view = surface.editorView
        let point = try cellPoint(surface, offset: 8)
        let shapes = session.provider.shapeBatchCallCount
        let copies = session.presentationExportCounters.geometryCopies
        let generation = surface.immutablePresentationGeneration
        for delta in 0..<100 { try move(view, to: NSPoint(x: point.x + CGFloat(delta % 12), y: point.y)) }
        XCTAssertNotNil(view.caretHoverRect)
        XCTAssertEqual(session.provider.shapeBatchCallCount, shapes)
        XCTAssertEqual(session.presentationExportCounters.geometryCopies, copies)
        XCTAssertEqual(surface.immutablePresentationGeneration, generation)
        _ = session.provider.invalidateMetrics()
        _ = try session.refreshLayoutIfNeeded()
        try move(view, to: point)
        XCTAssertNil(view.caretHoverRect, "A stale layout cannot supply a hover caret")
        surface.refreshPresentation()
        try move(view, to: point)
        XCTAssertNotNil(view.caretHoverRect)
    }

    private func assertPreviewMatchesClick(_ surface: EVEditorSurfaceController, at point: NSPoint) throws {
        let view = surface.editorView
        let session = try XCTUnwrap(surface.session)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let before = surface.viewPresentation
        let preview = try session.pointerCaret(view.layoutPoint(fromViewPoint: point), in: snapshot.info)
        try move(view, to: point)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, before.cursor_utf8_offset)
        XCTAssertEqual(surface.viewPresentation.mode, before.mode)
        XCTAssertEqual(surface.viewPresentation.visual_anchor_utf8_offset, before.visual_anchor_utf8_offset)
        if preview.flags & UInt32(VIEM_POINTER_CARET_CURRENT) != 0 { XCTAssertNil(view.caretHoverRect) }
        else { XCTAssertNotNil(view.caretHoverRect) }
        let rect = view.caretHoverRect
        view.mouseDown(with: try pointerEvent(view, type: .leftMouseDown, point: point))
        let after = surface.viewPresentation
        XCTAssertEqual(preview.mode, after.mode)
        XCTAssertEqual(preview.caret_shape, after.caret_shape)
        XCTAssertEqual(preview.text_start, after.caret_utf8_start)
        XCTAssertEqual(preview.text_end, after.caret_utf8_end)
        if let rect, preview.caret_shape == UInt32(VIEM_CARET_SHAPE_CELL), preview.mode != UInt32(VIEM_MODE_REPLACE) {
            XCTAssertEqual(rect, view.caretItemGeometry(try XCTUnwrap(surface.layoutSnapshot)).rect)
        }
        if let rect, preview.mode == UInt32(VIEM_MODE_INSERT) {
            XCTAssertEqual(rect.width, 2)
            XCTAssertEqual(rect, view.subviews.compactMap { $0 as? NSTextInsertionIndicator }.first?.frame)
        }
        XCTAssertNil(view.caretHoverRect)
        view.mouseUp(with: try pointerEvent(view, type: .leftMouseUp, point: point))
        try move(view, to: point)
        XCTAssertNil(view.caretHoverRect, "Hovering the just-clicked target must not duplicate the caret")
    }

    private func cellPoint(_ surface: EVEditorSurfaceController, offset: UInt64) throws -> NSPoint {
        let cluster = try XCTUnwrap(surface.layoutSnapshot?.clusters.first { $0.text_start == offset })
        return surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(cluster.x + cluster.advance * 0.5),
            y: CGFloat(cluster.typographic_bounds.y + cluster.typographic_bounds.height / 2)))
    }
    private func move(_ view: EVEditorView, to point: NSPoint) throws {
        view.mouseMoved(with: try pointerEvent(view, type: .mouseMoved, point: point))
    }
    private func pointerEvent(_ view: EVEditorView, type: NSEvent.EventType, point: NSPoint) throws -> NSEvent {
        try XCTUnwrap(NSEvent.mouseEvent(with: type, location: view.convert(point, to: nil), modifierFlags: [], timestamp: 0,
            windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 0))
    }
    private func makeSurface(_ source: String) throws -> (EVEditorSurfaceController, EVTestFocusWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-hover-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        surface.editorView.editingPreferences = EVEditingPreferences(configuration: EVConfigurationStore(directory: directory))
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 320), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 320)
        surface.viewDidLayout(); surface.refreshPresentation()
        surface.editorView.applicationIsActive = { true }
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        return (surface, window)
    }
}
