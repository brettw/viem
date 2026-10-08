import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVCodeScrollingTests: XCTestCase {
    func testIdleFontInvalidationIsRefreshedBeforeWheelAndScrollbarRequests() async throws {
        let (backend, surface, _) = try makeRustSurface()
        let session = try XCTUnwrap(surface.session)
        try await waitForRustPaint(backend: backend, surface: surface)

        for iteration in 0..<8 {
            _ = session.provider.invalidateMetrics()
            surface.editorView.scrollWheel(with: CodeWheelEvent(deltaY: -83))
            XCTAssertGreaterThan(surface.viewportState.top, 0, message(surface))
            XCTAssertNil(surface.commandOutput, "Iteration \(iteration): \(message(surface))")

            _ = session.provider.invalidateMetrics()
            surface.requestVerticalViewport(top: CGFloat(1_000 + iteration * 500))
            XCTAssertGreaterThan(surface.viewportState.top, Float(500 + iteration * 500), message(surface))
            XCTAssertNil(surface.commandOutput, "Iteration \(iteration): \(message(surface))")
            XCTAssertEqual(try session.layoutExport().info.identity.metrics_generation,
                           session.provider.metricsGeneration)
        }
    }

    func testRustScrollingPublishesSyntaxForNewVisibleRanges() async throws {
        let (backend, surface, source) = try makeRustSurface()
        let session = try XCTUnwrap(surface.session)
        try await waitForRustPaint(backend: backend, surface: surface)

        // Visit uncached syntax ranges, allowing the native timer to finish
        // any provider work that exceeded the bounded presentation wait.
        // Each viewport must actually acquire its own visible syntax paint.
        let destinations: [CGFloat] = [64, 512, 2_048, 8_192, 16_384, 32_768, 4_096, 0]
        for _ in 0..<2 {
            for top in destinations {
                let before = try session.viewportState()
                _ = try session.setViewportOrigin(left: 0, top: top, expected: before)
                surface.refreshPresentation()
                try await waitForVisibleSyntaxPaint(backend: backend, surface: surface)
                let after = try session.viewportState()
                let layout = try session.layoutExport()
                if top > CGFloat(before.top) {
                    XCTAssertGreaterThan(after.top, before.top)
                } else if top < CGFloat(before.top) {
                    XCTAssertLessThan(after.top, before.top)
                }
                if top == 0 { XCTAssertEqual(after.top, 0, accuracy: 0.1) }
                // Distant positions initially use estimated paragraph heights;
                // exact wrapping may move the y coordinate while retaining the
                // requested text anchor. The published viewport must be covered.
                XCTAssertLessThanOrEqual(layout.info.coverage_y_start, after.top)
                XCTAssertGreaterThanOrEqual(layout.info.coverage_y_end, after.top)
                XCTAssertEqual(layout.info.identity.layout_revision, after.layout_revision)
                XCTAssertEqual(layout.info.identity.configuration_generation, after.configuration_generation)
                XCTAssertNil(surface.commandOutput, message(surface))
            }
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
    }

    func testRustAndVimScrollingPublishesBeforeTheFirstFrameOrCompletesAfterTimeout() async throws {
        for (filename, text) in [
            ("scroll.rs", (0..<1_024).map { "fn item_\($0)() { let value = \($0); }\n" }.joined()),
            ("scroll.vim", (0..<1_024).map { "set number \" row \($0)\n" }.joined())
        ] {
            let source = Data(text.utf8)
            let (backend, surface, _) = try makeCodeSurface(filename: filename, source: source)
            // Finish cold provider setup before testing newly exposed coverage.
            // Inspect the first frame before polling or yielding to the timer.
            try await waitForVisibleSyntaxPaint(backend: backend, surface: surface)
            let start = ProcessInfo.processInfo.systemUptime
            surface.requestVerticalViewport(top: 8_192)
            if !hasVisibleSyntaxPaint(surface) {
                // Real provider latency varies by machine. A slow provider may
                // use the shared 100 ms grace period and finish asynchronously.
                XCTAssertGreaterThanOrEqual(ProcessInfo.processInfo.systemUptime - start, 0.08,
                    "\(filename) presented missing colors without waiting for syntax")
                try await waitForVisibleSyntaxPaint(backend: backend, surface: surface)
            }
            XCTAssertGreaterThan(surface.viewportState.top, 0)
            let session = try XCTUnwrap(surface.session)
            XCTAssertEqual(try session.layoutExport().info.identity.layout_revision,
                           surface.viewportState.layout_revision)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertFalse(surface.canUndo)
        }
    }

    func testRustWheelScrollingRemainsUsableAcrossSyntaxAndGeometryRefreshes() async throws {
        let (backend, surface, source) = try makeRustSurface()
        let session = try XCTUnwrap(surface.session)
        try await waitForRustPaint(backend: backend, surface: surface)

        for iteration in 0..<24 {
            if iteration % 4 == 0 {
                // A font/scale environment change and width change require
                // fresh geometry while syntax work is independently pending.
                _ = session.provider.invalidateMetrics()
                surface.view.frame.size.width = iteration % 8 == 0 ? 360 : 520
                surface.viewDidLayout()
            }
            XCTAssertNotNil(surface.layoutSnapshot, "Iteration \(iteration): \(message(surface))")
            let oldTextStart = try firstVisibleTextStart(surface)
            surface.editorView.scrollWheel(with: CodeWheelEvent(deltaY: -83))
            try await waitForVisibleSyntaxPaint(backend: backend, surface: surface)
            // Geometry refresh can change the absolute y coordinate. Forward
            // scrolling must advance through source text across that reflow.
            XCTAssertGreaterThan(try firstVisibleTextStart(surface), oldTextStart,
                                 "Iteration \(iteration): \(message(surface))")
            XCTAssertNil(surface.commandOutput, message(surface))
            do {
                let layout = try session.layoutExport()
                XCTAssertEqual(layout.info.identity.layout_revision, surface.viewportState.layout_revision)
                XCTAssertEqual(layout.info.identity.configuration_generation,
                               surface.viewportState.configuration_generation)
            } catch {
                XCTFail("Iteration \(iteration) could not export layout after scrolling: \(error)")
                return
            }
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    private func makeRustSurface() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, Data) {
        let source = Data((0..<4_096).map { index in
            if index >= 128 && index % 11 == 0 {
                // These glyphs first appear outside initial layout coverage and
                // exercise font fallback as new syntax-bearing rows are shaped.
                return "fn item_\(index)() { let message = \"العربية 日本語 👩🏽‍💻\"; } // café\n"
            }
            return "fn item_\(index)(argument: u64) -> u64 { let answer = argument + \(index); answer }\n"
        }.joined().utf8)
        return try makeCodeSurface(filename: "scroll.rs", source: source)
    }

    private func makeCodeSurface(
        filename: String, source: Data
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, Data) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-code-scroll-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        // SwiftPM's runner has no bundled fallback syntax resources. Use the
        // same runtime as the packaged app when Tree-sitter yields a time slice.
        var checkout = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { checkout.deleteLastPathComponent() }
        let resources = checkout.appendingPathComponent(".build/Viem.app/Contents/Resources")
        _ = try Data(contentsOf: resources.appendingPathComponent("vim/runtime/syntax/rust.vim"))
        let backend = EVCoreDocumentBackend(configuration:
            EVConfigurationStore(directory: directory, legacyDefaults: nil, bundleResourceURL: resources))
        try backend.read(source: source, typeName: EVDocument.plainTextType,
                         filename: filename, allowAutomaticCode: true)
        XCTAssertEqual(backend.sourceFormat, .code)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 420, height: 180)
        surface.viewDidLayout()
        return (backend, surface, source)
    }

    private func waitForRustPaint(
        backend: EVCoreDocumentBackend,
        surface: EVEditorSurfaceController
    ) async throws {
        for _ in 0..<200 {
            backend.pollSyntax()
            // Initial font registration may retire the first presentation
            // independently of the syntax worker's publication event.
            surface.refreshPresentation()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= 2
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rust keyword did not receive syntax paint: \(surface.statusBarState.message)")
    }

    private func waitForVisibleSyntaxPaint(
        backend: EVCoreDocumentBackend,
        surface: EVEditorSurfaceController
    ) async throws {
        for _ in 0..<200 {
            backend.pollSyntax()
            surface.refreshPresentation()
            if hasVisibleSyntaxPaint(surface) { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("New Code viewport did not receive syntax paint: \(surface.statusBarState.message)")
    }

    private func hasVisibleSyntaxPaint(_ surface: EVEditorSurfaceController) -> Bool {
        guard let layout = surface.layoutSnapshot else { return false }
        let rows = layout.rows.filter {
            $0.y + max($0.line_advance, 1) > surface.viewportState.top
                && $0.y < surface.viewportState.top + layout.info.viewport_height
        }
        guard let first = rows.first, let last = rows.last else { return false }
        return surface.layoutPaint?.runs.contains(where: {
            $0.text_end > first.text_start && $0.text_start < last.text_end
                && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
        }) == true
    }

    private func firstVisibleTextStart(_ surface: EVEditorSurfaceController) throws -> UInt64 {
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        let row = try XCTUnwrap(layout.rows.first(where: {
            $0.y + max($0.line_advance, 1) > surface.viewportState.top
        }))
        return row.text_start
    }

    private func message(_ surface: EVEditorSurfaceController) -> String {
        surface.commandOutput ?? surface.statusBarState.message
    }
}

private final class CodeWheelEvent: NSEvent {
    private let delta: CGFloat

    init(deltaY: CGFloat) {
        delta = deltaY
        super.init()
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError("init(coder:) is unavailable") }

    override var type: NSEvent.EventType { .scrollWheel }
    override var scrollingDeltaX: CGFloat { 0 }
    override var scrollingDeltaY: CGFloat { delta }
    override var hasPreciseScrollingDeltas: Bool { true }
    override var phase: NSEvent.Phase { [] }
    override var momentumPhase: NSEvent.Phase { [] }
}
