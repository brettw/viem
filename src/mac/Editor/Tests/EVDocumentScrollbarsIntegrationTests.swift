import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVDocumentScrollbarsIntegrationTests: XCTestCase {
    private func makeSurface(_ source: String, typeName: String = EVDocument.plainTextType) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: typeName)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 160)
        surface.viewDidLayout()
        return (backend, surface)
    }

    func testHorizontalScrollerUsesOnlyVisibleLinesAndResetsWhenTheyLeave() throws {
        let source = (0..<150).map { line in
            line == 30 ? String(repeating: "W", count: 100)
                : line == 130 ? String(repeating: "W", count: 500)
                : "Short line \(line)"
        }.joined(separator: "\n")
        let (backend, surface) = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        let bars = surface.editorView.documentScrollbars
        surface.performInput { _ = try session.setWrap(false) }
        XCTAssertFalse(bars.horizontalAvailable)
        XCTAssertEqual(bars.horizontalState.maximum, 0, accuracy: 0.01)

        surface.performInput { _ = try session.sendText("31Gzt") }
        XCTAssertTrue(bars.horizontalAvailable)
        let mediumMaximum = bars.horizontalState.maximum
        XCTAssertGreaterThan(mediumMaximum, 0)
        let caret = surface.viewPresentation.cursor_utf8_offset
        bars.performScrollAction(axis: .horizontal, part: .knob, value: 0.5)
        XCTAssertEqual(CGFloat(surface.viewportState.left), mediumMaximum / 2, accuracy: 0.1)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)

        surface.performInput { _ = try session.sendText("71Gzt") }
        XCTAssertFalse(bars.horizontalAvailable)
        XCTAssertEqual(surface.viewportState.left, 0, accuracy: 0.01)
        XCTAssertEqual(bars.horizontalState.maximum, 0, accuracy: 0.01)
        surface.performInput { _ = try session.sendText("131Gzt") }
        XCTAssertGreaterThan(bars.horizontalState.maximum, mediumMaximum * 3)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        XCTAssertNil(surface.commandOutput)
    }

    func testWrappedUnbreakableLineKeepsHorizontalScrollerAvailable() throws {
        let source = String(repeating: "W", count: 500)
        let (backend, surface) = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.setWrap(true) }
        let bars = surface.editorView.documentScrollbars
        XCTAssertNotEqual(surface.viewportState.flags & UInt32(VIEM_VIEWPORT_STATE_WRAP), 0)
        XCTAssertTrue(bars.horizontalAvailable)
        XCTAssertGreaterThan(bars.horizontalState.maximum, 0)
        let caret = surface.viewPresentation.cursor_utf8_offset
        let maximum = bars.horizontalState.maximum
        bars.performScrollAction(axis: .horizontal, part: .knob, value: 0.5)
        XCTAssertEqual(CGFloat(surface.viewportState.left), maximum / 2, accuracy: 0.1)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        XCTAssertNil(surface.commandOutput)
    }

    func testVerticalThumbAndPageActionsScrollWithoutMovingCaretOrEditing() throws {
        let source = (0..<400).map { "Paragraph \($0)" }.joined(separator: "\n")
        let (backend, surface) = try makeSurface(source)
        let bars = surface.editorView.documentScrollbars
        let caret = surface.viewPresentation.cursor_utf8_offset
        XCTAssertGreaterThan(bars.verticalState.maximum, 0)
        XCTAssertLessThan(bars.verticalScroller.knobProportion, 1)
        bars.performScrollAction(axis: .vertical, part: .incrementPage, value: 0)
        XCTAssertGreaterThan(surface.viewportState.top, 0)
        bars.performScrollAction(axis: .vertical, part: .knob, value: 1)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(snapshot.info.coverage_hard_line_end, 400)
        XCTAssertEqual(surface.viewportState.top,
            max(snapshot.info.coverage_y_start, snapshot.info.coverage_y_end - snapshot.info.viewport_height), accuracy: 0.1)
        bars.performScrollAction(axis: .vertical, part: .knob, value: 0)
        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        XCTAssertNil(surface.commandOutput)
    }

    func testScrollbarRefreshKeepsLargeDocumentExportsBounded() throws {
        let source = String(repeating: "A short paragraph.\n", count: 20_000)
        let (backend, surface) = try makeSurface(source)
        let bars = surface.editorView.documentScrollbars
        for fraction in [0.25, 0.75, 1, 0] {
            bars.performScrollAction(axis: .vertical, part: .knob, value: fraction)
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            XCTAssertEqual(bars.verticalState.maximum, CGFloat(surface.viewportState.maximum_top), accuracy: 0.01)
            XCTAssertLessThan(snapshot.rows.count, 100)
            XCTAssertFalse(bars.horizontalAvailable)
            XCTAssertNil(surface.commandOutput)
            if fraction == 1 {
                XCTAssertEqual(snapshot.info.coverage_hard_line_end, 20_001)
                XCTAssertNotEqual(surface.viewportState.flags & UInt32(VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT), 0)
                XCTAssertEqual(surface.viewportState.top, surface.viewportState.maximum_top, accuracy: 0.1)
                XCTAssertEqual(surface.viewportState.top,
                    max(0, snapshot.info.coverage_y_end - snapshot.info.viewport_height), accuracy: 0.1)
            }
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
    }

    func testMarkdownPageKeysAndScrollbarShareStablePaddedEndpoints() throws {
        let source = (0..<30).map { "# Heading \($0)\n\nA paragraph with enough words to wrap over more than one visual row.\n" }.joined(separator: "\n")
        let (backend, surface) = try makeSurface(source, typeName: EVDocument.markdownType)
        let session = try XCTUnwrap(surface.session)
        surface.performInput { try session.setViewMargins(.init(top: 17, left: 10, bottom: 29, right: 10)) }
        let bars = surface.editorView.documentScrollbars
        var previous = surface.viewportState.top
        var stationary = 0
        for _ in 0..<250 {
            let previousCursor = surface.viewPresentation.cursor_utf8_offset
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_PAGE_DOWN)) }
            XCTAssertGreaterThanOrEqual(surface.viewportState.top + 0.01, previous)
            stationary = surface.viewportState.top == previous
                && surface.viewPresentation.cursor_utf8_offset == previousCursor ? stationary + 1 : 0
            previous = surface.viewportState.top
            if stationary == 3 { break }
        }
        let bottom = surface.viewportState.top
        XCTAssertGreaterThan(bottom, 0)
        XCTAssertEqual(bottom, surface.viewportState.maximum_top, accuracy: 0.1)
        XCTAssertEqual(bars.verticalState.maximum, CGFloat(bottom), accuracy: 0.1)
        XCTAssertEqual(try XCTUnwrap(surface.layoutSnapshot).info.content_insets.bottom, 29)
        bars.performScrollAction(axis: .vertical, part: .knob, value: 1)
        XCTAssertEqual(surface.viewportState.top, bottom, accuracy: 0.1)
        stationary = 0
        for _ in 0..<250 {
            let previousCursor = surface.viewPresentation.cursor_utf8_offset
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_PAGE_UP)) }
            XCTAssertLessThanOrEqual(surface.viewportState.top - 0.01, previous)
            stationary = surface.viewportState.top == previous
                && surface.viewPresentation.cursor_utf8_offset == previousCursor ? stationary + 1 : 0
            previous = surface.viewportState.top
            if stationary == 3 { break }
        }
        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)
        bars.performScrollAction(axis: .vertical, part: .knob, value: 0)
        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertNil(surface.commandOutput)
    }

    func testNativeScrollbarAppearanceFollowsTheCanvasForContrast() throws {
        let original = EVThemeStore.shared.theme
        defer { EVThemeStore.shared.update(original) }
        EVThemeStore.shared.update(.paper)
        let (_, surface) = try makeSurface(String(repeating: "Line\n", count: 100))
        let bars = surface.editorView.documentScrollbars
        XCTAssertEqual(bars.appearance?.name, .aqua)
        XCTAssertEqual(bars.verticalScroller.knobStyle, .dark)
        EVThemeStore.shared.update(.midnight)
        XCTAssertEqual(bars.appearance?.name, .darkAqua)
        XCTAssertEqual(bars.verticalScroller.knobStyle, .light)
    }
}
