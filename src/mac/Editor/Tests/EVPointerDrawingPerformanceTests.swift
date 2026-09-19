import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemCoreTextProvider
@testable import ViemEditor

@MainActor
final class EVPointerDrawingPerformanceTests: XCTestCase {
    func testVisiblePointerSelectionUsesOneRefreshAndBoundedDamageInLargeDocument() throws {
        let source = String(repeating: "fifty riffraff with a tab\tand trailing spaces  \n", count: 20_000)
        let (surface, window) = try makeSurface(source)
        defer { withExtendedLifetime(window) {} }
        try pointer(surface, offset: 2, extending: false)
        let session = try XCTUnwrap(surface.session)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let identity = snapshot.info.identity
        let generation = surface.immutablePresentationGeneration
        let refreshes = surface.presentationRefreshCount
        let shapes = session.provider.shapeBatchCallCount
        let copies = session.presentationExportCounters.geometryCopies
        let image = try bitmap(for: surface.editorView)
        draw(surface.editorView, in: image, damage: [surface.editorView.bounds])
        for index in 0..<40 {
            try pointer(surface, offset: UInt64(4 + index % 15), extending: true)
            XCTAssertEqual(surface.presentationRefreshCount, refreshes + UInt64(index + 1))
            XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: identity))
            XCTAssertEqual(surface.immutablePresentationGeneration, generation)
            let damage = surface.editorView.presentationDamageRects
            XCTAssertFalse(damage.isEmpty)
            XCTAssertTrue(damage.allSatisfy { $0.height < surface.editorView.bounds.height / 3 })
            draw(surface.editorView, in: image, damage: damage)
        }
        XCTAssertEqual(session.provider.shapeBatchCallCount, shapes)
        XCTAssertEqual(session.presentationExportCounters.geometryCopies, copies)
        XCTAssertLessThan(snapshot.rows.count, 150)
        XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
    }

    func testPartialSelectionFramesMatchFreshRenderingAndProfileNativeDrawing() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let agents = try String(contentsOf: root.appendingPathComponent("AGENTS.md"), encoding: .utf8)
        let fixtures = [
            ("AGENTS.md", agents),
            ("large document", String(repeating: "fifty riffraff with a tab\tand trailing spaces  \n", count: 20_000)),
        ]
        for (label, source) in fixtures {
            let (surface, window) = try makeSurface(source)
            defer { withExtendedLifetime(window) {} }
            let view = surface.editorView
            try pointer(surface, offset: 2, extending: false)
            let retained = try bitmap(for: view)
            draw(view, in: retained, damage: [view.bounds])
            let fresh = try bitmap(for: view)
            var fullSeconds = 0.0
            var partialSeconds = 0.0
            var fullClusters = 0
            var partialClusters = 0
            for index in 0..<20 {
                try pointer(surface, offset: UInt64(4 + index % 7), extending: true)
                let damage = view.presentationDamageRects
                let snapshot = try XCTUnwrap(surface.layoutSnapshot)
                fullClusters += view.drawingClusters(in: view.bounds, snapshot: snapshot).count
                partialClusters += damage.reduce(0) { $0 + view.drawingClusters(in: $1, snapshot: snapshot).count }
                var start = CFAbsoluteTimeGetCurrent()
                draw(view, in: retained, damage: damage)
                partialSeconds += CFAbsoluteTimeGetCurrent() - start
                start = CFAbsoluteTimeGetCurrent()
                draw(view, in: fresh, damage: [view.bounds])
                fullSeconds += CFAbsoluteTimeGetCurrent() - start
                XCTAssertTrue(pixels(retained) == pixels(fresh), "Partial selection frame \(index) must match fresh drawing in \(label)")
            }
            XCTAssertLessThan(partialClusters, fullClusters / 2)
            print("MAC_DRAW_PROFILE \(label): 20 frames, full=\(fullSeconds * 1000)ms partial=\(partialSeconds * 1000)ms; cluster draws full=\(fullClusters) partial=\(partialClusters)")
        }
    }

    func testContentAndEnvironmentChangesInvalidateDamageAndMatchFreshRendering() throws {
        let originalTheme = EVThemeStore.shared.theme
        defer { EVThemeStore.shared.update(originalTheme) }
        let source = String(repeating: "fifty riffraff\twith spaces  \n", count: 100)
        let (surface, window) = try makeSurface(source)
        defer { withExtendedLifetime(window) {} }
        let view = surface.editorView
        let session = try XCTUnwrap(surface.session)
        let retained = try bitmap(for: view)
        draw(view, in: retained, damage: [view.bounds])
        func verify(_ label: String, _ change: () throws -> Void) throws {
            try change()
            XCTAssertEqual(view.presentationDamageRects, [view.bounds], "\(label) must repaint the complete viewport")
            draw(view, in: retained, damage: view.presentationDamageRects)
            let fresh = try bitmap(for: view)
            draw(view, in: fresh, damage: [view.bounds])
            XCTAssertTrue(pixels(retained) == pixels(fresh), "\(label) must match fresh rendering")
        }
        try verify("edit") { surface.performInput { _ = try session.sendText("x") } }
        try verify("undo") { surface.performInput { _ = try session.undo() } }
        try verify("redo") { surface.performInput { _ = try session.redo() } }
        try verify("resize") { _ = try session.resize(width: 300, height: 280); surface.refreshPresentation() }
        try verify("zoom") { surface.perform(menuCommand: .zoomIn, sender: nil) }
        try verify("font metrics") { _ = session.provider.invalidateMetrics(); surface.refreshPresentation() }
        try verify("whitespace") { surface.performInput { try session.setVisibleWhitespace(false) } }
        try verify("theme") { EVThemeStore.shared.update(originalTheme == .midnight ? .paper : .midnight) }
        try verify("viewport") { surface.requestVerticalViewport(top: 90) }
        try verify("appearance") {
            view.appearance = NSAppearance(named: .darkAqua)
            view.viewDidChangeEffectiveAppearance()
        }
        try verify("backing scale notification") { view.viewDidChangeBackingProperties(); view.applyPresentation() }
        surface.performInput { _ = try session.sendText("i") }
        draw(view, in: retained, damage: [view.bounds])
        try verify("composition") {
            view.setMarkedText("e\u{301}lan", selectedRange: NSRange(location: 2, length: 1),
                               replacementRange: NSRange(location: NSNotFound, length: 0))
        }
        XCTAssertTrue(view.hasMarkedText(), surface.statusBarState.message)
        try pointer(surface, offset: 3, extending: false)
        XCTAssertFalse(view.hasMarkedText())
        XCTAssertFalse(surface.statusBarState.message.localizedCaseInsensitiveContains("stale"))
    }

    func testPointerRefreshesExternallyChangedGeometryBeforeHitTesting() throws {
        let (surface, window) = try makeSurface("first words\nsecond words\nlast")
        defer { withExtendedLifetime(window) {} }
        let session = try XCTUnwrap(surface.session)
        let old = try XCTUnwrap(surface.layoutSnapshot).info.identity
        _ = session.provider.invalidateMetrics()
        // The provider can rebuild outside a presentation refresh. A false
        // refreshLayoutIfNeeded() result alone must not validate old geometry.
        _ = try session.refreshLayoutIfNeeded()
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: old))
        try pointer(surface, offset: 3, extending: false)
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: old))
        XCTAssertFalse(surface.statusBarState.message.localizedCaseInsensitiveContains("stale"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 3)
    }

    private func makeSurface(_ source: String) throws -> (EVEditorSurfaceController, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 320),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 320)
        surface.viewDidLayout()
        surface.refreshPresentation()
        surface.editorView.applicationIsActive = { true }
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        return (surface, window)
    }

    private func pointer(_ surface: EVEditorSurfaceController, offset: UInt64, extending: Bool) throws {
        let view = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let cluster = try XCTUnwrap(snapshot.clusters.first { $0.text_start == offset })
        let local = view.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(cluster.x + 0.1),
                                                           y: CGFloat(cluster.typographic_bounds.y + cluster.typographic_bounds.height / 2)))
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: extending ? .leftMouseDragged : .leftMouseDown,
            location: view.convert(local, to: nil), modifierFlags: [], timestamp: 0,
            windowNumber: windowNumber(view), context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        if extending { view.mouseDragged(with: event) } else { view.mouseDown(with: event) }
    }

    private func windowNumber(_ view: NSView) -> Int { view.window?.windowNumber ?? 0 }

    private func bitmap(for view: NSView) throws -> CGContext {
        try XCTUnwrap(CGContext(data: nil, width: Int(view.bounds.width), height: Int(view.bounds.height),
            bitsPerComponent: 8, bytesPerRow: Int(view.bounds.width) * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    }

    private func draw(_ view: EVEditorView, in context: CGContext, damage: [NSRect]) {
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        for rect in damage { view.draw(rect) }
        NSGraphicsContext.restoreGraphicsState()
    }

    private func pixels(_ context: CGContext) -> Data {
        Data(bytes: context.data!, count: context.bytesPerRow * context.height)
    }
}
