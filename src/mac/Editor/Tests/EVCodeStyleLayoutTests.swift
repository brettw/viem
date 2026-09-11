import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVCodeStyleLayoutTests: XCTestCase {
    func testKeywordFontEditsReshapeTwoRustBuffersWithoutChangingSourceOrDocumentHistory() async throws {
        let configuration = configuration()
        let fixtures = try (0..<2).map { try makeRustSurface(configuration: configuration, index: $0) }
        for fixture in fixtures { try await waitForKeywordSize(14, fixture: fixture) }
        let before = try fixtures.map { try $0.backend.recoverySnapshot() }
        let originalAdvance = try fixtures.map { try firstVisibleRow($0.surface).line_advance }
        let session = try EVCodeStyleSession(configuration: configuration)
        let keyword = try XCTUnwrap(session.snapshot().definitions.first { $0.name == "@keyword" })
        let editor = EVStyleEditorViewController()
        editor.retarget(codeSession: session)
        editor.selectStyle(keyword.key)

        editor.beginContinuousStyleEditForTesting()
        for (property, value) in [
            (EVStyleProperty.characterFontFamilies, EVStyleValue.stringList(["Georgia"])),
            (.characterSize, .float(30)),
            (.characterBold, .boolean(false)),
            (.characterWeight, .unsigned(400)),
            (.characterSlant, .fontSlant(0)),
        ] {
            XCTAssertTrue(editor.setPropertyForTesting(property, value: value), editor.inspection.diagnostic)
        }
        editor.endContinuousStyleEditForTesting()
        for (index, fixture) in fixtures.enumerated() {
            try await waitForKeywordSize(30, fixture: fixture)
            XCTAssertGreaterThan(try firstVisibleRow(fixture.surface).line_advance, originalAdvance[index] * 1.5)
            let layout = try XCTUnwrap(fixture.surface.layoutSnapshot)
            // The function name keeps the base size: this must be mixed-font
            // syntax layout, not a document-wide font-size replacement.
            let function = try XCTUnwrap(layout.clusters.first { $0.text_start == 3 })
            XCTAssertEqual(try fontSize(of: function, fixture: fixture), 14, accuracy: 0.01)
            try assertCurrentBoundedLayout(fixture)
        }

        let upright = try fixtures.map { try firstVisibleKeyword($0).render_run.identifier }
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(700)), editor.inspection.diagnostic)
        for (index, fixture) in fixtures.enumerated() {
            try await waitForKeywordSize(30, fixture: fixture)
            XCTAssertNotEqual(try firstVisibleKeyword(fixture).render_run.identifier, upright[index],
                              "Weight changes must replace the native glyph resource")
        }
        let bold = try fixtures.map { try firstVisibleKeyword($0).render_run.identifier }
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(1)), editor.inspection.diagnostic)
        for (index, fixture) in fixtures.enumerated() {
            try await waitForKeywordSize(30, fixture: fixture)
            XCTAssertNotEqual(try firstVisibleKeyword(fixture).render_run.identifier, bold[index],
                              "Slant changes must replace the native glyph resource")
        }

        editor.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertTrue(editor.setPropertyForTesting(.paragraphLineSpacing, value: .lineSpacing(
            EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: 48))), editor.inspection.diagnostic)
        for (index, fixture) in fixtures.enumerated() {
            fixture.backend.pollSyntax()
            fixture.surface.refreshPresentation()
            XCTAssertEqual(try firstVisibleRow(fixture.surface).line_advance, 48, accuracy: 0.1)
            try assertCurrentBoundedLayout(fixture)
            XCTAssertEqual(try fixture.backend.recoverySnapshot(), before[index])
            XCTAssertEqual(try fixture.backend.serializedSource(typeName: EVDocument.plainTextType), fixture.source)
            XCTAssertFalse(fixture.backend.persistenceState.isDirty)
            XCTAssertFalse(fixture.surface.canUndo)
            XCTAssertFalse(fixture.surface.canRedo)
        }
        XCTAssertTrue(session.undoManager.canUndo, "Only the global settings target owns style history")
    }

    func testDistantRustSyntaxAndFontChangesKeepExactVisibleGeometryAndEstimatedScrollbar() async throws {
        let configuration = configuration()
        let fixture = try makeRustSurface(configuration: configuration, index: 0, lineCount: 8_192)
        let viewSession = try XCTUnwrap(fixture.surface.session)
        let styleSession = try EVCodeStyleSession(configuration: configuration)
        let keyword = try XCTUnwrap(styleSession.snapshot().definitions.first { $0.name == "@keyword" })
        try styleSession.edit(key: keyword.key, expected: styleSession.snapshot().identity,
                              mutation: .setDeclaration(.characterSize, .float(28)))
        try await waitForKeywordSize(28, fixture: fixture)
        let before = try fixture.backend.recoverySnapshot()

        for (iteration, destination) in [12_000, 68_000, 118_000, 35_000].enumerated() {
            fixture.surface.requestVerticalViewport(top: CGFloat(destination))
            XCTAssertNil(fixture.surface.commandOutput, fixture.surface.statusBarState.message)
            let arrival = try firstVisibleRow(fixture.surface)
            let localOffset = arrival.y - fixture.surface.viewportState.top
            try await waitForKeywordSize(iteration == 0 ? 28 : Float(30 + (iteration - 1) * 4), fixture: fixture)
            let highlighted = try firstVisibleRow(fixture.surface)
            XCTAssertEqual(highlighted.hard_line_index, arrival.hard_line_index,
                           "Asynchronous syntax must preserve the requested text anchor")
            XCTAssertEqual(highlighted.y - fixture.surface.viewportState.top, localOffset, accuracy: 0.5)

            // Change the actual syntax font while additional wheel input is
            // arriving. Unvisited prefix heights may stay estimates, but the
            // current rows, glyph resources, and paint must agree exactly.
            let size = Float(30 + iteration * 4)
            try styleSession.edit(key: keyword.key, expected: styleSession.snapshot().identity,
                                  mutation: .setDeclaration(.characterSize, .float(size)))
            let oldLine = try firstVisibleRow(fixture.surface).hard_line_index
            fixture.surface.editorView.scrollWheel(with: CodeStyleWheelEvent(deltaY: -180))
            try await waitForKeywordSize(size, fixture: fixture)
            XCTAssertGreaterThan(try firstVisibleRow(fixture.surface).hard_line_index, oldLine)
            try assertCurrentBoundedLayout(fixture)
            let layout = try XCTUnwrap(fixture.surface.layoutSnapshot)
            XCTAssertEqual(layout.info.flags & UInt32(VIEM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT), 0,
                           "Styling visible rows must not force an exact whole-document scrollbar height")
            XCTAssertGreaterThan(layout.info.total_height, layout.info.coverage_y_end)
            XCTAssertNil(fixture.surface.commandOutput, fixture.surface.statusBarState.message)
        }
        XCTAssertEqual(try fixture.backend.recoverySnapshot(), before)
        XCTAssertEqual(try fixture.backend.serializedSource(typeName: EVDocument.plainTextType), fixture.source)
        XCTAssertFalse(fixture.backend.persistenceState.isDirty)
        XCTAssertFalse(fixture.surface.canUndo)
        XCTAssertFalse(fixture.surface.canRedo)
        _ = try viewSession.layoutExport()
    }

    private struct Fixture {
        let backend: EVCoreDocumentBackend
        let surface: EVEditorSurfaceController
        let source: Data
    }

    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-style-layout-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func makeRustSurface(configuration: EVConfigurationStore, index: Int, lineCount: Int = 2_048) throws -> Fixture {
        let source = Data((0..<lineCount).map { "fn item_\(index)_\($0)() { let value = \($0); }\n" }.joined().utf8)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: source, typeName: EVDocument.plainTextType,
                         filename: "styles\(index).rs", allowAutomaticCode: true)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 560, height: 180)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        _ = try session.setWrap(false)
        surface.refreshPresentation()
        return Fixture(backend: backend, surface: surface, source: source)
    }

    private func firstVisibleRow(_ surface: EVEditorSurfaceController) throws -> ViemVisualRowV1 {
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        return try XCTUnwrap(layout.rows.first {
            $0.y + $0.line_advance > surface.viewportState.top + 0.01
        })
    }

    private func firstVisibleKeyword(_ fixture: Fixture) throws -> ViemPositionedClusterV1 {
        let row = try firstVisibleRow(fixture.surface)
        return try XCTUnwrap(fixture.surface.layoutSnapshot?.clusters.first { $0.text_start == row.text_start })
    }

    private func fontSize(of cluster: ViemPositionedClusterV1, fixture: Fixture) throws -> CGFloat {
        let provider = try XCTUnwrap(fixture.surface.session).provider
        return try XCTUnwrap(provider.renderRegistry.enAdvance(identifier: cluster.render_run.identifier,
                                                                metricsGeneration: cluster.render_run.metrics_generation)) * 2
    }

    private func waitForKeywordSize(_ size: Float, fixture: Fixture) async throws {
        for _ in 0..<300 {
            fixture.backend.pollSyntax()
            fixture.surface.refreshPresentation()
            if let cluster = try? firstVisibleKeyword(fixture),
               let resolved = try? fontSize(of: cluster, fixture: fixture),
               abs(resolved - CGFloat(size)) < 0.01,
               fixture.surface.layoutPaint?.runs.contains(where: {
                   $0.text_start <= cluster.text_start && $0.text_end > cluster.text_start
                       && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
               }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Visible Rust keyword did not acquire its \(size)-point font: \(fixture.surface.statusBarState.message)")
    }

    private func assertCurrentBoundedLayout(_ fixture: Fixture, file: StaticString = #filePath, line: UInt = #line) throws {
        let session = try XCTUnwrap(fixture.surface.session)
        let layout = try session.layoutExport()
        let viewport = try session.viewportState()
        let paint = try XCTUnwrap(fixture.surface.layoutPaint)
        XCTAssertEqual(layout.info.identity.layout_revision, viewport.layout_revision, file: file, line: line)
        XCTAssertEqual(layout.info.identity.configuration_generation, viewport.configuration_generation, file: file, line: line)
        XCTAssertEqual(layout.info.identity.metrics_generation, session.provider.metricsGeneration, file: file, line: line)
        XCTAssertTrue(paint.info.identity.isSameLayout(as: layout.info.identity), file: file, line: line)
        XCTAssertLessThanOrEqual(layout.info.coverage_y_start, viewport.top, file: file, line: line)
        XCTAssertGreaterThanOrEqual(layout.info.coverage_y_end, viewport.top + layout.info.viewport_height, file: file, line: line)
        XCTAssertLessThan(layout.rows.count, 100, "Native exact geometry must stay near the viewport", file: file, line: line)
        XCTAssertLessThan(layout.clusters.count, 6_000, file: file, line: line)
        XCTAssertTrue(layout.clusters.allSatisfy {
            session.provider.renderRegistry.contains(identifier: $0.render_run.identifier,
                                                     metricsGeneration: $0.render_run.metrics_generation)
        }, "Every displayed cluster must retain a current native glyph resource", file: file, line: line)
    }
}

private final class CodeStyleWheelEvent: NSEvent {
    private let delta: CGFloat
    init(deltaY: CGFloat) { delta = deltaY; super.init() }
    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError("init(coder:) is unavailable") }
    override var type: NSEvent.EventType { .scrollWheel }
    override var scrollingDeltaX: CGFloat { 0 }
    override var scrollingDeltaY: CGFloat { delta }
    override var hasPreciseScrollingDeltas: Bool { true }
    override var phase: NSEvent.Phase { [] }
    override var momentumPhase: NSEvent.Phase { [] }
}
