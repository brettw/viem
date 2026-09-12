import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

/// The drawn caret must occupy exactly what the core says it occupies.
///
/// `$` and `<End>` leave the model on the last character with an upstream
/// boundary affinity. A drawing layer that treated that affinity as a
/// character selector drew the block on the next-to-last character, so the
/// caret appeared stuck: one `h` produced no visible movement and `l` looked
/// blocked with a character still to the right.
@MainActor
final class EVCaretCellTests: XCTestCase {
    func testEndOfLineDrawsTheLastCharacterCellNotTheOneBeforeIt() throws {
        let source = "abcdef\nnext"
        let (surface, window) = try makeSurface(source)
        defer { withExtendedLifetime(window) {} }
        let session = try XCTUnwrap(surface.session)

        for key in ["$", "<End>"] {
            type(surface, "0")
            if key == "$" {
                type(surface, "$")
            } else {
                surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_END)) }
                surface.refreshPresentation()
            }
            let presentation = surface.viewPresentation
            XCTAssertEqual(presentation.cursor_utf8_offset, 5, key)
            XCTAssertEqual(presentation.cursor_affinity, UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM), key)
            XCTAssertEqual(EVCaretTarget(presentation), .cell(5 ..< 6), key)

            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let geometry = surface.editorView.caretItemGeometry(snapshot)
            let cluster = try XCTUnwrap(geometry.cluster, key)
            XCTAssertEqual(cluster.text_start, 5, key)
            XCTAssertEqual(cluster.text_end, 6, key)
            // The drawn cell is the `f` cell, strictly right of the `e` cell.
            let previous = try XCTUnwrap(snapshot.clusters.first { $0.text_start == 4 }, key)
            let rect = try XCTUnwrap(geometry.rect, key)
            XCTAssertGreaterThan(rect.minX, CGFloat(previous.x), key)
        }
    }

    func testEveryNormalModePositionDrawsTheClusterUnderTheCursor() throws {
        let source = "abc def\nsecond line"
        let (surface, window) = try makeSurface(source)
        defer { withExtendedLifetime(window) {} }
        for motion in ["0", "$", "w", "b", "e", "j", "j$", "G", "gg"] {
            type(surface, "gg0")
            type(surface, motion)
            let presentation = surface.viewPresentation
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let geometry = surface.editorView.caretItemGeometry(snapshot)
            guard case let .cell(range) = EVCaretTarget(presentation) else {
                XCTFail("Normal mode addresses characters: \(motion)")
                continue
            }
            let cluster = try XCTUnwrap(geometry.cluster, motion)
            XCTAssertLessThanOrEqual(cluster.text_start, range.lowerBound, motion)
            XCTAssertGreaterThan(cluster.text_end, range.lowerBound, motion)
            XCTAssertEqual(presentation.cursor_utf8_offset, range.lowerBound, motion)
        }
    }

    func testInsertModeDrawsAnInsertionBoundary() throws {
        let (surface, window) = try makeSurface("abcdef\nnext")
        defer { withExtendedLifetime(window) {} }
        type(surface, "$i")
        let presentation = surface.viewPresentation
        XCTAssertEqual(presentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(EVCaretTarget(presentation).offset, 5)
        guard case .boundary = EVCaretTarget(presentation) else {
            return XCTFail("Insert mode addresses gaps between characters")
        }
    }

    /// Drive input the way AppKit does: printable keys arrive through
    /// `insertText` even in a command mode, and the view normalizes them.
    private func type(_ surface: EVEditorSurfaceController, _ text: String) {
        for character in text {
            surface.editorView.insertText(
                String(character), replacementRange: NSRange(location: NSNotFound, length: 0))
        }
        surface.refreshPresentation()
    }

    private func makeSurface(_ source: String) throws -> (EVEditorSurfaceController, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.contentViewController = surface
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        surface.refreshPresentation()
        return (surface, window)
    }
}
