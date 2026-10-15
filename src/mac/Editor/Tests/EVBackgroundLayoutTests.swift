import AppKit
import CViemCore
import ImageIO
import ViemAppShell
import XCTest
@testable import ViemCoreTextProvider
@testable import ViemEditor

@MainActor
final class EVBackgroundLayoutTests: XCTestCase {
    private let source = (0..<10_000).map {
        "Paragraph \($0): **office** and *words* مرحبا 👩‍💻 that wrap into multiple visual rows.\n\n"
    }.joined()

    private func fixture(enabled: Bool = true, source override: String? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-prelayout-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data((override ?? source).utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        session.backgroundLayout.isEnabled = enabled
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        surface.refreshPresentation()
        return (backend, surface, session)
    }

    /// Text and screen offset of each row in the viewport.
    private func visibleRows(_ surface: EVEditorSurfaceController) throws -> [[Int]] {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let top = surface.viewportState.top
        return snapshot.rows.filter { $0.y + max($0.line_advance, 1) > top && $0.y < top + snapshot.info.viewport_height }
            .map { [Int($0.text_start), Int($0.text_end), Int((($0.y - top) * 64).rounded())] }
    }

    private func idle(_ scheduler: EVBackgroundLayout) async throws {
        for _ in 0..<1500 {
            if scheduler.isIdle { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(scheduler.isIdle)
        XCTAssertNil(scheduler.lastError)
    }

    func testSmallDocumentIsPreparedCompletelyAndScrollsWithoutShapingOrExtentChanges() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-prepared-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let figure = directory.appendingPathComponent("figure.png")
        let context = try XCTUnwrap(CGContext(data: nil, width: 640, height: 320, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: 640, height: 320))
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(figure as CFURL, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), nil)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        var source = ""
        for section in 0..<90 {
            source += "## Section \(section)\n\nParagraph \(section) with **bold** and *italic* words that wrap across the view.\n\n"
            source += "| Name | Value | Notes |\n| --- | ---: | --- |\n| one | \(section) | first |\n| two | \(section * 7) | second note |\n\n"
            if section % 30 == 15 { source += "![figure](\(figure.path))\n\n" }
        }
        XCTAssertLessThan(source.utf8.count, 100 * 1024)
        let (_, surface, session) = try fixture(source: source)
        try await idle(session.backgroundLayout)
        // The exact extent is published once preparation is quiet.
        try await Task.sleep(nanoseconds: 300_000_000)
        surface.refreshPresentation()
        XCTAssertGreaterThan(session.provider.imageDimensionProbeCountForTesting, 0, "Figures are measured before they are visible")
        let maximum = surface.viewportState.maximum_top
        XCTAssertGreaterThan(maximum, 0)
        let shapes = session.provider.shapeBatchCallCount
        var top: CGFloat = 0
        while top <= CGFloat(maximum) {
            surface.requestVerticalViewport(top: top)
            XCTAssertEqual(surface.viewportState.maximum_top, maximum, "The scroll extent stays exact at \(top)")
            top += 157
        }
        XCTAssertEqual(session.provider.shapeBatchCallCount, shapes, "Prepared text, tables and figures never shape on the input thread")
    }

    func testQueuedWorkCancelsForEditsMetricsResizeAndClose() async throws {
        let (backend, surface, session) = try fixture(enabled: false)
        EVBackgroundLayout.worker.suspend()
        var suspended = true
        defer { if suspended { EVBackgroundLayout.worker.resume() } }
        session.backgroundLayout.isEnabled = true
        session.backgroundLayout.update()
        for _ in 0..<100 {
            if session.backgroundLayout.started > 0 { break }
            await Task.yield()
        }
        XCTAssertGreaterThan(session.backgroundLayout.started, 0)
        surface.view.frame.size = NSSize(width: 330, height: 240)
        surface.viewDidLayout()
        surface.performInput {
            _ = try session.sendTestKeys("ggiChanged ")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        }
        session.provider.invalidateMetrics()
        surface.refreshPresentation()
        let visible = try visibleRows(surface)
        EVBackgroundLayout.worker.resume(); suspended = false
        try await idle(session.backgroundLayout)
        XCTAssertGreaterThan(session.backgroundLayout.discarded, 0)
        // Current work may publish exact extents, never move visible text.
        surface.refreshPresentation()
        XCTAssertEqual(try visibleRows(surface), visible)
        XCTAssertEqual(try session.layoutSnapshotInfo().identity.metrics_generation, session.provider.metricsGeneration)
        XCTAssertTrue(try backend.formattedText().hasPrefix("Changed "))
        EVBackgroundLayout.worker.suspend(); suspended = true
        let started = session.backgroundLayout.started
        surface.view.frame.size = NSSize(width: 340, height: 250)
        surface.viewDidLayout()
        for _ in 0..<100 {
            if session.backgroundLayout.started > started { break }
            await Task.yield()
        }
        XCTAssertGreaterThan(session.backgroundLayout.started, started)
        surface.detachFromCore()
        EVBackgroundLayout.worker.resume(); suspended = false
        try await idle(session.backgroundLayout)
        XCTAssertEqual(session.provider.renderRegistry.resourceCountForTesting, 0)
    }

}
