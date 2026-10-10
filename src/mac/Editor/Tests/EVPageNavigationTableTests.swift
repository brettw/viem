import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVPageNavigationTableTests: XCTestCase {
    private func pageKey(down: Bool) throws -> NSEvent {
        let character = down ? "\u{f72d}" : "\u{f72c}"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.function],
            timestamp: 0, windowNumber: 0, context: nil, characters: character,
            charactersIgnoringModifiers: character, isARepeat: true, keyCode: down ? 121 : 116))
    }

    private func surface(_ source: String,
                         declarations: [(EVStyleKey, EVStyleProperty, EVStyleValue?)] = []) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-table-pages-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .paragraphLineSpacing,
             .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: 24))),
            (.baseParagraph, .blockMarginTop, .float(0)),
            (.baseParagraph, .blockMarginBottom, .float(8)),
            (EVStyleTestFixtures.block("Heading2"), .characterSize, .float(24)),
            (EVStyleTestFixtures.block("Heading2"), .paragraphLineSpacing,
             .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: 32))),
            (EVStyleTestFixtures.block("Table cell"), .characterSize, .float(14)),
            (EVStyleTestFixtures.block("Table header"), .characterSize, .float(14)),
        ] + declarations)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 500)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func assertCaretVisible(_ surface: EVEditorSurfaceController,
                                    session: EVCoreViewSession, label: String) throws {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot, label)
        let presentation = surface.viewPresentation
        let viewport = try session.viewportState()
        let geometry = try session.caretGeometry(offset: presentation.cursor_utf8_offset,
            affinity: presentation.cursor_affinity, in: snapshot.info)
        XCTAssertGreaterThanOrEqual(geometry.rect.y, viewport.top - 0.1, label)
        XCTAssertLessThanOrEqual(geometry.rect.y + geometry.rect.height,
                                viewport.top + snapshot.info.viewport_height + 0.1, label)
        XCTAssertEqual(session.lastOutcome.command_status, UInt32(VIEM_COMMAND_STATUS_COMPLETE), label)
        XCTAssertNil(surface.commandOutput, label)
    }

    func testPhysicalPageKeysRetraceViewportAcrossTableAndProseWithoutExtraCaretScrolls() throws {
        let opening = "## Before the table\n\n" + (0..<8).map {
            "Opening paragraph \($0) has **formatted words** and ordinary prose.\n\n"
        }.joined()
        let table = "| Item | Description | Status |\n| --- | --- | --- |\n" + (0..<40).map {
            "| item \($0) | Short text \($0) | ready |\n"
        }.joined()
        let closing = "\n## After the table\n\n" + (0..<180).map {
            "Following paragraph \($0) keeps paging away from the document end.\n\n"
        }.joined()
        let source = opening + table + closing
        let (backend, surface, session) = try surface(source)
        defer { session.detach() }
        let revision = try backend.revision()
        let initial = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertLessThan(initial.info.coverage_hard_line_end, initial.info.document_hard_line_count,
                          "The first page should use the ordinary partial layout")
        let down = try pageKey(down: true), up = try pageKey(down: false)
        var tops = [try session.viewportState().top]
        var distance: Float?
        var tablePages = 0
        var prosePages = 0
        for step in 0..<9 {
            let before = try session.viewportState().top
            surface.editorView.keyDown(with: down)
            let after = try session.viewportState().top
            let delta = after - before
            let label = "Page Down \(step): \(before) → \(after)"
            if let distance { XCTAssertEqual(delta, distance, accuracy: 0.1, label) }
            else {
                distance = delta
                XCTAssertGreaterThan(delta, initial.info.viewport_height / 2, label)
                XCTAssertLessThan(delta, initial.info.viewport_height, label)
            }
            tops.append(after)
            let context = try session.tableContext()
            if context.flags & UInt32(VIEM_TABLE_IN_TABLE) != 0 { tablePages += 1 }
            else { prosePages += 1 }
            try assertCaretVisible(surface, session: session, label: label)
            surface.refreshPresentation()
            XCTAssertEqual(try session.viewportState().top, after, accuracy: 0.1,
                           "An ordinary presentation refresh must not scroll again")
        }
        XCTAssertGreaterThan(tablePages, 1, "Exercise multiple screens of side-by-side table cells")
        XCTAssertGreaterThan(prosePages, 1, "Exercise transitions to and from ordinary prose")
        for step in (0..<9).reversed() {
            let before = try session.viewportState().top
            surface.editorView.keyDown(with: up)
            let after = try session.viewportState().top
            let label = "Page Up \(step): \(before) → \(after)"
            XCTAssertEqual(after, tops[step], accuracy: 0.1, label)
            XCTAssertEqual(before - after, try XCTUnwrap(distance), accuracy: 0.1, label)
            try assertCaretVisible(surface, session: session, label: label)
        }
        XCTAssertEqual(try backend.revision(), revision)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.canRedo)
    }

    func testPagingInTablePreservesMiddleColumnAndUsesEqualDistancesInBothDirections() throws {
        let source = "| Left | Middle | Right |\n| --- | --- | --- |\n" + (0..<100).map {
            "| left \($0) | middle \($0) | right \($0) |\n"
        }.joined()
        let (_, surface, session) = try surface(source)
        defer { session.detach() }
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let cell = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 && $0.column == 1 })
        let geometry = try session.caretGeometry(offset: cell.text_start,
            affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
        surface.performInput { _ = try session.placeCursor(geometry.point, extendSelection: false) }
        let top = try session.viewportState().top
        let down = try pageKey(down: true), up = try pageKey(down: false)
        var distances: [Float] = []
        for (step, event) in [down, down, up, up].enumerated() {
            let before = try session.viewportState().top
            surface.editorView.keyDown(with: event)
            let after = try session.viewportState().top
            distances.append(abs(after - before))
            let context = try session.tableContext()
            XCTAssertNotEqual(context.flags & UInt32(VIEM_TABLE_IN_TABLE), 0, "step \(step)")
            XCTAssertEqual(context.column, 1, "Paging follows vertical geometry rather than adjacent cell entries")
            try assertCaretVisible(surface, session: session, label: "middle-column page \(step)")
        }
        for distance in distances { XCTAssertEqual(distance, distances[0], accuracy: 0.1) }
        XCTAssertEqual(try session.viewportState().top, top, accuracy: 0.1)
    }

    func testOversizedRowsPreferAnAvailableVisibleBaselineWithoutChangingPageDistance() throws {
        let source = (0..<12).map { "## \($0)\n\n" }.joined()
        let (backend, surface, session) = try surface(source, declarations: [
            (EVStyleTestFixtures.block("Heading2"), .characterSize, .float(100)),
            (EVStyleTestFixtures.block("Heading2"), .paragraphLineSpacing,
             .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: 50))),
            (EVStyleTestFixtures.block("Heading2"), .blockMarginTop, .float(0)),
            (EVStyleTestFixtures.block("Heading2"), .blockMarginBottom, .float(0)),
        ])
        defer { session.detach() }
        let original = try XCTUnwrap(surface.layoutSnapshot)
        let first = try XCTUnwrap(original.rows.first)
        // Keep the original baseline near the bottom of a small viewport.
        // Exact spacing makes the oversized neighboring rows overlap, so a
        // nearest baseline can lie offscreen even when another remains usable.
        let screenBaseline = min(first.baseline, 90)
        surface.view.frame.size.height = CGFloat(screenBaseline + 5)
        surface.viewDidLayout()
        surface.performInput { _ = try session.setViewportOrigin(left: 0, top: CGFloat(first.baseline - screenBaseline)) }
        let before = try session.viewportState().top
        let beforeCursor = surface.viewPresentation.cursor_utf8_offset
        surface.editorView.keyDown(with: try pageKey(down: true))
        let after = try session.viewportState().top
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let bottom = after + snapshot.info.viewport_height
        let visible = snapshot.rows.filter { $0.y < bottom && $0.y + $0.line_advance > after }
        XCTAssertFalse(visible.isEmpty)
        XCTAssertTrue(visible.allSatisfy {
            $0.y < after || $0.y + $0.ascent + $0.descent + $0.leading > bottom
        }, "The fixture must require the oversized-row fallback")
        let ideal = after + first.baseline - before
        let nearest = try XCTUnwrap(visible.min { abs($0.baseline - ideal) < abs($1.baseline - ideal) })
        XCTAssertTrue(nearest.baseline < after || nearest.baseline > bottom,
                      "The nearest oversized row must have an offscreen baseline")
        XCTAssertTrue(visible.contains { after <= $0.baseline && $0.baseline <= bottom },
                      "Another visible row must offer an accessible baseline")
        let actual = try XCTUnwrap(snapshot.rows.first { row in
            snapshot.carets.contains { $0.row_index == row.row_index
                && $0.text_offset == surface.viewPresentation.cursor_utf8_offset
                && $0.affinity == surface.viewPresentation.cursor_affinity }
        })
        XCTAssertGreaterThanOrEqual(actual.baseline, after)
        XCTAssertLessThanOrEqual(actual.baseline, bottom)
        XCTAssertEqual(session.lastOutcome.command_status, UInt32(VIEM_COMMAND_STATUS_COMPLETE))
        XCTAssertNil(surface.commandOutput)
        XCTAssertGreaterThan(after, before)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, beforeCursor,
                       "Retain the usable row rather than selecting its offscreen neighbor")
        surface.editorView.keyDown(with: try pageKey(down: false))
        XCTAssertEqual(try session.viewportState().top, before, accuracy: 0.1,
                       "Oversized-row caret fallback must not add a second scroll correction")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }
}
