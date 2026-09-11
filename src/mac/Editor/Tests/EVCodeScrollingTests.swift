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

        // Visit uncached syntax ranges and explicitly run the same publication
        // path as the native timer. Scrolling alone need not poll highlighting;
        // each new viewport must actually acquire its own visible syntax paint.
        let destinations: [CGFloat] = [64, 512, 2_048, 8_192, 16_384, 32_768, 4_096, 0]
        for _ in 0..<2 {
            for top in destinations {
                allowSyntaxWorkerToFinish()
                let before = try session.viewportState()
                _ = try session.setViewportOrigin(left: 0, top: top, expected: before)
                surface.refreshPresentation()
                try await waitForVisibleRustPaint(backend: backend, surface: surface)
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

    func testRustWheelScrollingRemainsUsableAcrossSyntaxAndGeometryRefreshes() async throws {
        let (backend, surface, source) = try makeRustSurface()
        let session = try XCTUnwrap(surface.session)
        try await waitForRustPaint(backend: backend, surface: surface)

        for iteration in 0..<24 {
            allowSyntaxWorkerToFinish()
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
            try await waitForVisibleRustPaint(backend: backend, surface: surface)
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
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-code-scroll-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration:
            EVConfigurationStore(directory: directory, legacyDefaults: nil))
        let source = Data((0..<4_096).map { index in
            if index >= 128 && index % 11 == 0 {
                // These glyphs first appear outside initial layout coverage and
                // exercise font fallback as new syntax-bearing rows are shaped.
                return "fn item_\(index)() { let message = \"العربية 日本語 👩🏽‍💻\"; } // café\n"
            }
            return "fn item_\(index)(argument: u64) -> u64 { let answer = argument + \(index); answer }\n"
        }.joined().utf8)
        try backend.read(source: source, typeName: EVDocument.plainTextType,
                         filename: "scroll.rs", allowAutomaticCode: true)
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

    private func allowSyntaxWorkerToFinish() {
        Thread.sleep(forTimeInterval: 0.025)
    }

    private func waitForVisibleRustPaint(
        backend: EVCoreDocumentBackend,
        surface: EVEditorSurfaceController
    ) async throws {
        for _ in 0..<200 {
            backend.pollSyntax()
            surface.refreshPresentation()
            if let layout = surface.layoutSnapshot {
                let rows = layout.rows.filter {
                    $0.y + max($0.line_advance, 1) > surface.viewportState.top
                        && $0.y < surface.viewportState.top + layout.info.viewport_height
                }
                if let first = rows.first, let last = rows.last,
                   surface.layoutPaint?.runs.contains(where: {
                       $0.text_end > first.text_start && $0.text_start < last.text_end
                           && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
                   }) == true { return }
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("New Rust viewport did not receive syntax paint: \(surface.statusBarState.message)")
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
