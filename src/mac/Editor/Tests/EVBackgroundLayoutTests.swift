import AppKit
import CViemCore
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

    private func idle(_ scheduler: EVBackgroundLayout) async throws {
        for _ in 0..<1500 {
            if scheduler.isIdle { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(scheduler.isIdle)
        XCTAssertNil(scheduler.lastError)
    }

    private func pixels(_ view: EVEditorView) throws -> Data {
        let context = try XCTUnwrap(CGContext(data: nil, width: Int(view.bounds.width), height: Int(view.bounds.height),
            bitsPerComponent: 8, bytesPerRow: Int(view.bounds.width) * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        view.draw(view.bounds)
        NSGraphicsContext.restoreGraphicsState()
        return Data(bytes: context.data!, count: context.bytesPerRow * context.height)
    }

    func testWorkerWarmsThreePagesWithoutChangingVisiblePresentation() async throws {
        let (backend, surface, session) = try fixture()
        let before = try session.viewportState()
        let cursor = surface.viewPresentation.cursor_utf8_offset
        let exports = session.presentationExportCounters
        let initialPixels = try pixels(surface.editorView)
        let foreground = session.provider.shapeBatchCallCount
        await Task.yield()
        try await idle(session.backgroundLayout)
        XCTAssertGreaterThan(session.backgroundLayout.installed, 0)
        XCTAssertEqual(session.backgroundLayout.workerComputations, session.backgroundLayout.started)
        XCTAssertLessThanOrEqual(session.backgroundLayout.started, 32)
        XCTAssertEqual(session.provider.shapeBatchCallCount, foreground)
        let after = try session.viewportState()
        XCTAssertEqual(after.top, before.top)
        XCTAssertEqual(after.layout_revision, before.layout_revision)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(session.presentationExportCounters.geometryCopies, exports.geometryCopies)
        XCTAssertEqual(try pixels(surface.editorView), initialPixels)
        let started = session.backgroundLayout.started
        surface.performInput { _ = try session.sendText("l") }
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(session.backgroundLayout.started, started, "Caret motion must not restart idle work")
        let beforePages = session.provider.shapeBatchCallCount
        for _ in 0..<3 { surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_PAGE_DOWN)) } }
        XCTAssertEqual(session.provider.shapeBatchCallCount, beforePages, "Three prepared pages require no foreground shaping")
        let (coldBackend, cold, coldSession) = try fixture(enabled: false)
        cold.performInput { _ = try coldSession.sendText("l") }
        for _ in 0..<3 { cold.performInput { _ = try coldSession.sendKey(kind: UInt32(VIEM_KEY_PAGE_DOWN)) } }
        XCTAssertEqual(try pixels(surface.editorView), try pixels(cold.editorView), "Worker glyphs match foreground Core Text pixels")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        withExtendedLifetime(coldBackend) { cold.detachFromCore() }
        surface.detachFromCore()
        try await idle(session.backgroundLayout)
        XCTAssertEqual(session.provider.renderRegistry.resourceCountForTesting, 0)
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
            _ = try session.sendText("ggiChanged ")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        }
        session.provider.invalidateMetrics()
        surface.refreshPresentation()
        let current = try session.viewportState()
        EVBackgroundLayout.worker.resume(); suspended = false
        try await idle(session.backgroundLayout)
        XCTAssertGreaterThan(session.backgroundLayout.discarded, 0)
        XCTAssertEqual(try session.viewportState().layout_revision, current.layout_revision)
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

    func testCompositionPausesWorkAndCancellationResumesWithoutEditingSource() async throws {
        let (backend, surface, session) = try fixture(enabled: false)
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.setMarkedText("uncommitted", selectedRange: NSRange(location: 11, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(session.hasActiveComposition)
        session.backgroundLayout.isEnabled = true
        surface.refreshPresentation()
        try await idle(session.backgroundLayout)
        XCTAssertEqual(session.backgroundLayout.started, 0)
        surface.editorView.cancelOperation(nil)
        XCTAssertFalse(session.hasActiveComposition)
        try await idle(session.backgroundLayout)
        XCTAssertGreaterThan(session.backgroundLayout.installed, 0)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        surface.detachFromCore()
        try await idle(session.backgroundLayout)
        XCTAssertEqual(session.provider.renderRegistry.resourceCountForTesting, 0)
    }

    func testIdleGlyphRetentionDoesNotGrowWithUnrelatedDocumentSuffix() async throws {
        var resources: [Int] = []
        var bytes: [Int] = []
        for paragraphs in [1_000, 10_000] {
            let input = source.split(separator: "\n", omittingEmptySubsequences: true)
                .prefix(paragraphs).joined(separator: "\n\n") + "\n\n"
            let (backend, surface, session) = try fixture(source: input)
            try await idle(session.backgroundLayout)
            XCTAssertLessThanOrEqual(session.backgroundLayout.started, 32)
            resources.append(session.provider.renderRegistry.resourceCountForTesting)
            bytes.append(session.provider.renderRegistry.estimatedBytesForTesting)
            withExtendedLifetime(backend) { surface.detachFromCore() }
            try await idle(session.backgroundLayout)
            XCTAssertEqual(session.provider.renderRegistry.resourceCountForTesting, 0)
        }
        XCTAssertGreaterThan(resources[0], 0)
        XCTAssertEqual(resources[0], resources[1])
        XCTAssertEqual(bytes[0], bytes[1])
    }

    func testOrdinaryAndRapidPagingCompareTheSameNativeFixture() async throws {
        for rapid in [false, true] {
            var calls: [UInt64] = []
            var rendered: [Data] = []
            for enabled in [false, true] {
                let (backend, surface, session) = try fixture(enabled: enabled)
                if enabled { try await idle(session.backgroundLayout) }
                let before = session.provider.shapeBatchCallCount
                var inputTime = Duration.zero
                for _ in 0..<12 {
                    let start = ContinuousClock.now
                    surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_PAGE_DOWN)) }
                    inputTime += start.duration(to: .now)
                    if !rapid && enabled { try await idle(session.backgroundLayout) }
                }
                print("prelayout paging rapid=\(rapid) enabled=\(enabled): \(inputTime), foreground batches=\(session.provider.shapeBatchCallCount - before)")
                calls.append(session.provider.shapeBatchCallCount - before)
                rendered.append(try pixels(surface.editorView))
                withExtendedLifetime(backend) { surface.detachFromCore() }
                try await idle(session.backgroundLayout)
            }
            XCTAssertEqual(rendered[0], rendered[1])
            if rapid {
                // An uninterrupted burst can outrun worker installation; input
                // never waits for it and must not do extra foreground shaping.
                XCTAssertGreaterThanOrEqual(calls[0], calls[1])
            } else {
                XCTAssertGreaterThan(calls[0], calls[1])
                XCTAssertEqual(calls[1], 0)
            }
        }
    }
}
