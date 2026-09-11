import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVCodeEditingStabilityTests: XCTestCase {
    private let implicitRange = NSRange(location: NSNotFound, length: 0)

    func testTypingInScrolledCodePreservesOffCenterCaretBaselineWithAndWithoutRustSyntax() async throws {
        for highlighting in [false, true] {
            let fixture = try makeFixture(highlighting: highlighting)
            if highlighting { try await waitForCommentSize(14, fixture: fixture) }
            fixture.surface.requestVerticalViewport(top: 18_000)
            if highlighting { try await waitForCommentSize(14, fixture: fixture) }
            try placeInsertCaret(fixture, screenFraction: 0.3)
            let baseline = try caretState(fixture).screenBaseline
            let viewportHeight = try XCTUnwrap(fixture.surface.layoutSnapshot).info.viewport_height
            XCTAssertGreaterThan(abs(baseline - viewportHeight / 2), 12,
                                 "The fixture must detect accidental recentering")
            let insertion = Int(fixture.surface.viewPresentation.cursor_utf8_offset)
            let typed = "reliable"
            for character in typed {
                let client: NSTextInputClient = fixture.surface.editorView
                client.insertText(String(character), replacementRange: implicitRange)
                try assertBaseline(baseline, fixture: fixture,
                                   reason: "Immediate typing of \(character), Rust=\(highlighting)")
                // Source-change callbacks and the real syntax timer can both
                // refresh the same view after the immediate editing turn.
                try await Task.sleep(for: .milliseconds(20))
                fixture.backend.pollSyntax()
                fixture.surface.refreshPresentation()
                try assertBaseline(baseline, fixture: fixture,
                                   reason: "Deferred refresh after \(character), Rust=\(highlighting)")
            }
            var expected = fixture.source
            expected.insert(contentsOf: typed.utf8, at: insertion)
            XCTAssertEqual(try fixture.backend.serializedSource(typeName: EVDocument.plainTextType), expected)
            if !highlighting { XCTAssertTrue(try fixture.backend.syntaxStyleNames().isEmpty) }
            try assertCurrentBoundedLayout(fixture)
        }
    }

    func testDelayedLargerRustSyntaxPreservesTheTypedCaretBaseline() async throws {
        let fixture = try makeFixture(highlighting: true, commentSize: 36, includeComments: false)
        try await waitForCommentSize(14, fixture: fixture)
        fixture.surface.requestVerticalViewport(top: 24_000)
        try await waitForCommentSize(14, fixture: fixture)
        try placeInsertCaret(fixture, screenFraction: 0.4, atLineStart: true)
        let before = try caretState(fixture)
        XCTAssertLessThan(before.row.ascent, 20)
        // Editing a delimiter makes this line a comment. Its 36-point syntax
        // style arrives through the real worker/poll path after the edit.
        let client: NSTextInputClient = fixture.surface.editorView
        client.insertText("//", replacementRange: implicitRange)
        try assertBaseline(before.screenBaseline, fixture: fixture, reason: "Typing before syntax publication")
        let afterTyping = try caretState(fixture)

        try await waitForCaretCommentSize(36, fixture: fixture)
        let styled = try caretState(fixture)
        XCTAssertGreaterThan(styled.row.ascent, 20,
                             "The caret row must actually receive the large syntax font")
        XCTAssertEqual(styled.screenBaseline, afterTyping.screenBaseline, accuracy: 0.5,
                       "The baseline must stay fixed when deferred syntax changes ascent and prefix heights")
        XCTAssertGreaterThanOrEqual(styled.row.y - fixture.surface.viewportState.top, -0.5)
        XCTAssertLessThanOrEqual(styled.row.y + styled.row.line_advance - fixture.surface.viewportState.top,
                                try XCTUnwrap(fixture.surface.layoutSnapshot).info.viewport_height + 0.5)
        try assertCurrentBoundedLayout(fixture)
    }

    func testLargerSyntaxMovesCaretOnlyEnoughToExposeItsWholeRow() async throws {
        let fixture = try makeFixture(highlighting: true)
        try await waitForCommentSize(14, fixture: fixture)
        fixture.surface.requestVerticalViewport(top: 18_000)
        try await waitForCommentSize(14, fixture: fixture)
        try placeInsertCaret(fixture, screenFraction: 0.3)
        var state = try caretState(fixture)
        fixture.surface.requestVerticalViewport(top: CGFloat(state.row.baseline - state.row.ascent - 1))
        state = try caretState(fixture)
        let priorBaseline = state.screenBaseline
        let client: NSTextInputClient = fixture.surface.editorView
        client.insertText("x", replacementRange: implicitRange)
        let session = try EVCodeStyleSession(configuration: fixture.configuration)
        let snapshot = try session.snapshot()
        let comment = try XCTUnwrap(snapshot.definitions.first { $0.name == "Comment" })
        try session.edit(key: comment.key, expected: snapshot.identity,
                         mutation: .setDeclaration(.characterSize, .float(36)))
        try await waitForCaretCommentSize(36, fixture: fixture)

        let styled = try caretState(fixture)
        let height = try XCTUnwrap(fixture.surface.layoutSnapshot).info.viewport_height
        let ascentFromRowTop = styled.row.baseline - styled.row.y
        let belowBaseline = styled.row.line_advance - ascentFromRowTop
        let nearestVisibleBaseline = min(max(priorBaseline, ascentFromRowTop), height - belowBaseline)
        XCTAssertEqual(styled.screenBaseline, nearestVisibleBaseline, accuracy: 0.5,
                       "Only the smallest scroll needed to reveal the taller caret row is allowed")
        XCTAssertEqual(styled.row.y - fixture.surface.viewportState.top, 0, accuracy: 0.5)
        try assertCurrentBoundedLayout(fixture)
    }

    private struct Fixture {
        let configuration: EVConfigurationStore
        let backend: EVCoreDocumentBackend
        let surface: EVEditorSurfaceController
        let source: Data
    }

    private struct CaretState {
        let row: ViemVisualRowV1
        let screenBaseline: Float
    }

    private func makeFixture(highlighting: Bool, commentSize: Float = 14,
                             includeComments: Bool = true) throws -> Fixture {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-edit-stability-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        if !highlighting { try configuration.setVimSyntaxDirectory(directory.appendingPathComponent("missing-syntax").path) }
        let styles = try EVCodeStyleSession(configuration: configuration)
        if commentSize != 14 {
            let snapshot = try styles.snapshot()
            let comment = try XCTUnwrap(snapshot.definitions.first { $0.name == "Comment" })
            try styles.edit(key: comment.key, expected: snapshot.identity,
                            mutation: .setDeclaration(.characterSize, .float(commentSize)))
        }
        let comment = includeComments ? " // note" : ""
        let source = Data((0..<4_096).map { "fn item_\($0)() { let value = \($0); }\(comment)\n" }.joined().utf8)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: source, typeName: EVDocument.codeType,
                         filename: highlighting ? "editing.rs" : "editing.unknown-code-syntax", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 680, height: 240)
        surface.viewDidLayout()
        _ = try XCTUnwrap(surface.session).setWrap(false)
        surface.refreshPresentation()
        return Fixture(configuration: configuration, backend: backend, surface: surface, source: source)
    }

    private func placeInsertCaret(_ fixture: Fixture, screenFraction: Float, atLineStart: Bool = false) throws {
        let surface = fixture.surface
        let session = try XCTUnwrap(surface.session)
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        let targetY = surface.viewportState.top + layout.info.viewport_height * screenFraction
        let row = try XCTUnwrap(layout.rows.first { $0.y + $0.line_advance > targetY })
        let geometry = try session.caretGeometry(offset: atLineStart ? row.text_start : row.text_end - 1,
            affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: layout.info)
        surface.performInput { _ = try session.placeCursor(geometry.point, extendSelection: false) }
        let client: NSTextInputClient = surface.editorView
        client.insertText("i", replacementRange: implicitRange)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
    }

    private func caretState(_ fixture: Fixture) throws -> CaretState {
        let surface = fixture.surface
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        let session = try XCTUnwrap(surface.session)
        let presentation = surface.viewPresentation
        let geometry = try session.caretGeometry(offset: presentation.cursor_utf8_offset,
            affinity: presentation.cursor_affinity, in: layout.info)
        let row = try XCTUnwrap(layout.rows.first { $0.row_index == geometry.row_index })
        return CaretState(row: row, screenBaseline: row.baseline - surface.viewportState.top)
    }

    private func assertBaseline(_ expected: Float, fixture: Fixture, reason: String,
                                file: StaticString = #filePath, line: UInt = #line) throws {
        XCTAssertEqual(try caretState(fixture).screenBaseline, expected, accuracy: 0.5,
                       reason, file: file, line: line)
        XCTAssertNil(fixture.surface.commandOutput, fixture.surface.statusBarState.message, file: file, line: line)
    }

    private func commentSize(in row: ViemVisualRowV1, fixture: Fixture) throws -> CGFloat {
        let layout = try XCTUnwrap(fixture.surface.layoutSnapshot)
        let cluster = try XCTUnwrap(layout.clusters.last {
            $0.row_index == row.row_index && $0.text_start < row.text_end
        })
        let provider = try XCTUnwrap(fixture.surface.session).provider
        return try XCTUnwrap(provider.renderRegistry.enAdvance(identifier: cluster.render_run.identifier,
            metricsGeneration: cluster.render_run.metrics_generation)) * 2
    }

    private func waitForCommentSize(_ expected: CGFloat, fixture: Fixture) async throws {
        for _ in 0..<200 {
            fixture.backend.pollSyntax()
            fixture.surface.refreshPresentation()
            if let layout = fixture.surface.layoutSnapshot,
               let row = layout.rows.first(where: { $0.y + $0.line_advance > fixture.surface.viewportState.top }),
               let size = try? commentSize(in: row, fixture: fixture), abs(size - expected) < 0.01,
               fixture.surface.layoutPaint?.runs.contains(where: {
                   $0.text_start < row.text_end && $0.text_end > row.text_start
                       && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
               }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Visible Rust comments did not acquire \(expected)-point syntax styling")
    }

    private func waitForCaretCommentSize(_ expected: CGFloat, fixture: Fixture) async throws {
        for _ in 0..<200 {
            fixture.backend.pollSyntax()
            fixture.surface.refreshPresentation()
            if let caret = try? caretState(fixture),
               let size = try? commentSize(in: caret.row, fixture: fixture), abs(size - expected) < 0.01 { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Caret row did not acquire \(expected)-point syntax styling")
    }

    private func assertCurrentBoundedLayout(_ fixture: Fixture) throws {
        let session = try XCTUnwrap(fixture.surface.session)
        let layout = try session.layoutExport()
        XCTAssertEqual(layout.info.identity.layout_revision, fixture.surface.viewportState.layout_revision)
        XCTAssertEqual(layout.info.identity.metrics_generation, session.provider.metricsGeneration)
        XCTAssertLessThan(layout.rows.count, 150)
        XCTAssertLessThan(layout.clusters.count, 8_000)
        XCTAssertNil(fixture.surface.commandOutput, fixture.surface.statusBarState.message)
    }
}
