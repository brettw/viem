import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemCoreTextProvider
@testable import ViemEditor

@MainActor
final class EVPresentationCacheIntegrationTests: XCTestCase {
    func testRepeatedVisibleSelectionsInAgentFileReusePresentationExports() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let text = try String(contentsOf: root.appendingPathComponent("AGENTS.md"), encoding: .utf8)
        try assertSelectionsReuseExports(text, label: "AGENTS.md", type: EVDocument.markdownSourceType)
    }

    func testRepeatedVisibleSelectionsInTwentyThousandLinesReusePresentationExports() throws {
        let text = (0..<20_000).map { "Line \($0): selection caching keeps visible text ready.  " }
            .joined(separator: "\n")
        try assertSelectionsReuseExports(text, label: "20,000 lines")
    }

    func testEditUndoAndRedoInvalidateExportsAndVisibleText() throws {
        let (backend, surface, session) = try fixture("Alpha beta\nsecond line  ")
        let initial = try XCTUnwrap(surface.layoutSnapshot)
        let before = session.presentationExportCounters
        surface.performInput {
            _ = try session.sendText("iChanged ")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        }
        let edited = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(edited.info.identity.isSameLayout(as: initial.info.identity))
        XCTAssertGreaterThan(session.presentationExportCounters.geometryCopies, before.geometryCopies)
        XCTAssertEqual(try backend.formattedText(), "Changed Alpha beta\nsecond line  ")
        try assertFreshMatchesPresentation(surface, session: session)

        surface.performInput { _ = try session.undo() }
        XCTAssertEqual(try backend.formattedText(), "Alpha beta\nsecond line  ")
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: edited.info.identity))
        try assertFreshMatchesPresentation(surface, session: session)

        surface.performInput { _ = try session.redo() }
        XCTAssertEqual(try backend.formattedText(), "Changed Alpha beta\nsecond line  ")
        try assertFreshMatchesPresentation(surface, session: session)
    }

    func testResizeZoomAndMetricsInvalidateExports() throws {
        let (_, surface, session) = try fixture(String(repeating: "Alpha beta gamma delta ", count: 20))
        _ = try session.setWrap(true)
        surface.refreshPresentation()
        var previous = try XCTUnwrap(surface.layoutSnapshot).info.identity

        surface.view.frame.size = NSSize(width: 330, height: 210)
        surface.viewDidLayout()
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: previous))
        try assertFreshMatchesPresentation(surface, session: session)
        previous = try XCTUnwrap(surface.layoutSnapshot).info.identity

        surface.perform(menuCommand: .zoomIn, sender: nil)
        XCTAssertGreaterThan(surface.zoomScale, 1)
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: previous))
        try assertFreshMatchesPresentation(surface, session: session)
        previous = try XCTUnwrap(surface.layoutSnapshot).info.identity

        session.provider.invalidateMetrics()
        surface.refreshPresentation()
        let current = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(current.info.identity.isSameLayout(as: previous))
        XCTAssertEqual(current.info.identity.metrics_generation, session.provider.metricsGeneration)
        XCTAssertTrue(current.clusters.allSatisfy {
            session.provider.renderRegistry.contains(identifier: $0.render_run.identifier,
                metricsGeneration: $0.render_run.metrics_generation)
        })
        try assertFreshMatchesPresentation(surface, session: session)
    }

    func testWhitespaceOptionsAndViewportInvalidateOnlyCurrentMarkerExport() throws {
        let (_, surface, session) = try fixture(String(repeating: "word ", count: 100) + "  ")
        _ = try session.setWrap(false)
        surface.refreshPresentation()
        let original = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(original.whitespace.markers.isEmpty)
        var malformedIdentity = original.info.identity
        malformedIdentity.struct_size = 0
        XCTAssertThrowsError(try session.whitespaceMarkersExport(identity: malformedIdentity))
        let viewport = try session.viewportState()
        let oldViewport = ViemLayoutRectV1(x: viewport.left, y: viewport.top,
            width: original.info.viewport_width, height: original.info.viewport_height)
        let copies = session.presentationExportCounters

        _ = try session.setViewportOrigin(left: 30, expected: viewport)
        // The caller's old cache key must be rejected even before another export
        // has replaced the cached marker batch with the new viewport.
        XCTAssertThrowsError(try session.whitespaceMarkersExport(
            identity: original.info.identity, expectedViewport: oldViewport))
        surface.refreshPresentation()
        let scrolled = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertTrue(scrolled.info.identity.isSameLayout(as: original.info.identity))
        XCTAssertNotEqual(scrolled.whitespace.markers, original.whitespace.markers)
        XCTAssertEqual(session.presentationExportCounters.geometryCopies, copies.geometryCopies)
        XCTAssertEqual(session.presentationExportCounters.paintCopies, copies.paintCopies)
        XCTAssertGreaterThan(session.presentationExportCounters.whitespaceCopies, copies.whitespaceCopies)
        try assertFreshMatchesPresentation(surface, session: session)

        try session.setVisibleWhitespace(false)
        XCTAssertThrowsError(try session.whitespaceMarkersExport(identity: scrolled.info.identity))
        surface.refreshPresentation()
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).whitespace.markers.isEmpty)
        try assertFreshMatchesPresentation(surface, session: session)
        try session.setVisibleWhitespace(true)
        surface.refreshPresentation()
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).whitespace.markers.isEmpty)
        try assertFreshMatchesPresentation(surface, session: session)
    }

    func testCompositionRefreshReusesSlicesAndUpdateAndCancellationInvalidateThem() throws {
        let (backend, surface, session) = try fixture("base  ")
        surface.performInput { _ = try session.sendText("A") }
        let revision = try backend.revision()
        surface.editorView.setMarkedText("first  ", selectedRange: NSRange(location: 7, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(session.hasActiveComposition)
        let initial = try XCTUnwrap(surface.layoutSnapshot)
        let counters = session.presentationExportCounters
        let shapeCalls = session.provider.shapeBatchCallCount
        backend.resetFormattedAccessCounters()
        for _ in 0..<20 { surface.refreshPresentation() }
        assertCopyCountsEqual(session.presentationExportCounters, counters)
        XCTAssertEqual(session.provider.shapeBatchCallCount, shapeCalls)
        XCTAssertEqual(backend.formattedAccessCounters.rangeReadCalls, 0)
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: initial.info.identity))
        try assertFreshMatchesPresentation(surface, session: session)

        surface.editorView.setMarkedText("second\t ", selectedRange: NSRange(location: 8, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertGreaterThan(session.presentationExportCounters.compositionTextCopies, counters.compositionTextCopies)
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: initial.info.identity))
        XCTAssertEqual(try backend.revision(), revision)
        try assertFreshMatchesPresentation(surface, session: session)

        surface.editorView.cancelOperation(nil)
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertTrue(surface.compositionTextSlices.isEmpty)
        XCTAssertFalse(surface.layoutTextSlices.isEmpty)
        XCTAssertEqual(try backend.formattedText(), "base  ")
        try assertFreshMatchesPresentation(surface, session: session)
    }

    func testNamedPaintChangeAndDecorationsMatchFreshExports() throws {
        let (_, surface, session) = try fixture("1. Alpha\n2. Beta\n", type: EVDocument.markdownType)
        XCTAssertFalse(try XCTUnwrap(surface.layoutSnapshot).decorations.isEmpty)
        let previous = try XCTUnwrap(surface.layoutPaint)
        let editor = EVStyleEditorViewController()
        editor.retarget(document: surface, styleKey: .baseParagraph)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground,
            value: .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1))), editor.inspection.diagnostic)
        let current = try XCTUnwrap(surface.layoutPaint)
        XCTAssertFalse(current.info.identity.isSameLayout(as: previous.info.identity))
        XCTAssertEqual(current.info.default_paint.foreground.red, 1)
        XCTAssertEqual(current.info.default_paint.foreground.green, 0)
        try assertFreshMatchesPresentation(surface, session: session)
    }

    private func assertSelectionsReuseExports(_ text: String, label: String,
        type: String = "public.plain-text") throws {
        let (backend, surface, session) = try fixture(text, type: type)
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        let row = try XCTUnwrap(layout.rows.first { $0.text_end > $0.text_start + 8 })
        let start = try session.hitTest(CGPoint(x: CGFloat(row.paragraph_content_x + 15),
            y: CGFloat(row.baseline)), in: layout.info)
        let end = try session.hitTest(CGPoint(x: CGFloat(row.paragraph_content_x + 100),
            y: CGFloat(row.baseline)), in: layout.info)
        XCTAssertNotEqual(start.text_offset, end.text_offset)
        _ = try session.placeCursor(start, extendSelection: false)
        _ = try session.placeCursor(end, extendSelection: true)
        surface.refreshPresentation()
        let identity = try XCTUnwrap(surface.layoutSnapshot).info.identity
        let copies = session.presentationExportCounters
        let shapeCalls = session.provider.shapeBatchCallCount
        backend.resetFormattedAccessCounters()
        var coreSeconds: TimeInterval = 0
        var refreshSeconds: TimeInterval = 0
        var exportSeconds: TimeInterval = 0
        for _ in 0..<40 {
            for (point, extending) in [(start, false), (end, true)] {
                let coreStart = ProcessInfo.processInfo.systemUptime
                _ = try session.placeCursor(point, extendSelection: extending)
                coreSeconds += ProcessInfo.processInfo.systemUptime - coreStart
                let refreshStart = ProcessInfo.processInfo.systemUptime
                surface.refreshPresentation()
                refreshSeconds += ProcessInfo.processInfo.systemUptime - refreshStart
                let exportStart = ProcessInfo.processInfo.systemUptime
                _ = try session.layoutExport()
                _ = try session.layoutPaintExport()
                exportSeconds += ProcessInfo.processInfo.systemUptime - exportStart
                XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: identity))
            }
        }
        assertCopyCountsEqual(session.presentationExportCounters, copies)
        XCTAssertEqual(backend.formattedAccessCounters.rangeReadCalls, 0)
        XCTAssertEqual(backend.formattedAccessCounters.requestedUTF8Bytes, 0)
        XCTAssertEqual(session.provider.shapeBatchCallCount, shapeCalls)
        XCTAssertFalse(try XCTUnwrap(surface.visualSelection).rectangles.isEmpty)
        XCTAssertLessThan(try XCTUnwrap(surface.layoutSnapshot).rows.count, 200)
        print(String(format: "Presentation cache [%@], 80 selections: core %.3f ms, refresh %.3f ms, cached exports %.3f ms",
            label, coreSeconds * 1000, refreshSeconds * 1000, exportSeconds * 1000))
        try assertFreshMatchesPresentation(surface, session: session)
    }

    private func fixture(_ text: String, type: String = "public.plain-text") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-presentation-cache-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(text.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func assertFreshMatchesPresentation(_ surface: EVEditorSurfaceController,
        session: EVCoreViewSession, file: StaticString = #filePath, line: UInt = #line) throws {
        let cached = try XCTUnwrap(surface.layoutSnapshot, file: file, line: line)
        let paint = try XCTUnwrap(surface.layoutPaint, file: file, line: line)
        let counters = session.presentationExportCounters
        _ = try session.layoutExport()
        _ = try session.layoutPaintExport()
        assertCopyCountsEqual(session.presentationExportCounters, counters, file: file, line: line)
        session.clearPresentationExportCache()
        let fresh = try session.layoutExport()
        let freshPaint = try session.layoutPaintExport()
        // C structs have no Equatable conformance. Reflection compares every
        // exported field, including identities and handles, without comparing
        // uninitialized C padding bytes.
        XCTAssertEqual(String(reflecting: cached.info), String(reflecting: fresh.info), file: file, line: line)
        XCTAssertEqual(cached.rows.map { String(reflecting: $0) }, fresh.rows.map { String(reflecting: $0) }, file: file, line: line)
        XCTAssertEqual(cached.clusters.map { String(reflecting: $0) }, fresh.clusters.map { String(reflecting: $0) }, file: file, line: line)
        XCTAssertEqual(cached.carets.map { String(reflecting: $0) }, fresh.carets.map { String(reflecting: $0) }, file: file, line: line)
        XCTAssertEqual(cached.decorations.map { String(reflecting: $0) }, fresh.decorations.map { String(reflecting: $0) }, file: file, line: line)
        XCTAssertEqual(cached.decorationLabels, fresh.decorationLabels, file: file, line: line)
        XCTAssertEqual(cached.whitespace, fresh.whitespace, file: file, line: line)
        XCTAssertEqual(String(reflecting: paint.info), String(reflecting: freshPaint.info), file: file, line: line)
        XCTAssertEqual(paint.runs.map { String(reflecting: $0) }, freshPaint.runs.map { String(reflecting: $0) }, file: file, line: line)
        XCTAssertEqual(session.presentationExportCounters.geometryCopies, counters.geometryCopies + 1, file: file, line: line)
        XCTAssertEqual(session.presentationExportCounters.paintCopies, counters.paintCopies + 1, file: file, line: line)
        XCTAssertEqual(session.presentationExportCounters.whitespaceCopies, counters.whitespaceCopies + 1, file: file, line: line)
        if let overlay = surface.compositionOverlay {
            for slice in surface.compositionTextSlices {
                XCTAssertEqual(slice.bytes, try session.compositionTextSlice(in: slice.utf8Range, overlay: overlay).bytes,
                    file: file, line: line)
            }
        } else {
            let snapshot = try XCTUnwrap(surface.formattedSnapshot, file: file, line: line)
            for slice in surface.layoutTextSlices {
                XCTAssertEqual(slice.bytes, try surface.backend.formattedSlice(in: slice.utf8Range, snapshot: snapshot).bytes,
                    file: file, line: line)
            }
        }
    }

    private func assertCopyCountsEqual(_ actual: EVPresentationExportCounters,
        _ expected: EVPresentationExportCounters, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(actual.geometryCopies, expected.geometryCopies, file: file, line: line)
        XCTAssertEqual(actual.paintCopies, expected.paintCopies, file: file, line: line)
        XCTAssertEqual(actual.whitespaceCopies, expected.whitespaceCopies, file: file, line: line)
        XCTAssertEqual(actual.compositionTextCopies, expected.compositionTextCopies, file: file, line: line)
    }
}
