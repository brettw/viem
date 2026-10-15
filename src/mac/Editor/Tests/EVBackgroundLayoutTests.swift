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

}
