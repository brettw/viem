import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVHTMLEOFEditingTests: XCTestCase {
    private let implicit = NSRange(location: NSNotFound, length: 0)

    private func makeSurface(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.frame = NSRect(x: 0, y: 0, width: 320, height: 240)
        _ = try XCTUnwrap(surface.session).resize(width: 320, height: 240)
        surface.refreshPresentation()
        return (backend, surface)
    }

    func testHyphenAfterListExitKeepsEmptyRowsEditableAndArrowsMoveOneRow() throws {
        for source in ["<ul><li>first</li></ul>", "<ol><li>first</li></ol>"] {
            let (backend, surface) = try makeSurface(source)
            let editor = surface.editorView
            let session = try XCTUnwrap(surface.session)
            editor.insertText("GA", replacementRange: implicit)
            for _ in 0..<3 { editor.doCommand(by: #selector(NSResponder.insertNewline(_:))) }
            editor.insertText("-", replacementRange: implicit)
            XCTAssertNil(surface.commandOutput, source)
            XCTAssertEqual(surface.statusBarState.message, "", source)
            XCTAssertEqual(surface.formattedText, "first\n\n-", source)
            let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
            let reopened = EVCoreDocumentBackend()
            try reopened.read(source: saved, typeName: EVDocument.htmlType)
            XCTAssertEqual(try reopened.formattedText(), "first\n\n-", source)

            for scale: CGFloat in [1, 2] {
                _ = try session.setScale(scale)
                surface.refreshPresentation()
                var previousY = try caretY(surface)
                for _ in 0..<2 {
                    editor.doCommand(by: #selector(NSResponder.moveUp(_:)))
                    let nextY = try caretY(surface)
                    XCTAssertLessThan(nextY, previousY, source)
                    previousY = nextY
                }
                for _ in 0..<2 {
                    editor.doCommand(by: #selector(NSResponder.moveDown(_:)))
                    let nextY = try caretY(surface)
                    XCTAssertGreaterThan(nextY, previousY, source)
                    previousY = nextY
                }
                XCTAssertNil(surface.commandOutput, source)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), saved)
            }
            editor.cancelOperation(nil)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    func testHyphenBeforeInvisibleTrailingElementsPreservesTheirSource() throws {
        for tail in ["<span></span>", "<div></div>", "<ul></ul>", "<!--keep--><span data-keep='yes'></span>"] {
            let source = "<p>first</p>" + tail
            let (backend, surface) = try makeSurface(source)
            surface.editorView.insertText("GA", replacementRange: implicit)
            surface.editorView.insertText("-", replacementRange: implicit)
            XCTAssertEqual(surface.formattedText, "first-", source)
            XCTAssertNil(surface.commandOutput, source)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(("<p>first-</p>" + tail).utf8))
            surface.editorView.cancelOperation(nil)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    private func caretY(_ surface: EVEditorSurfaceController) throws -> Float {
        let session = try XCTUnwrap(surface.session)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let presentation = surface.viewPresentation
        // At EOF there may only be an upstream shaping caret. Use the same
        // affinity fallback as the editor's insertion-indicator drawing.
        let requested = try? session.caretGeometry(offset: presentation.cursor_utf8_offset,
            affinity: presentation.cursor_affinity, in: snapshot.info)
        let alternate = presentation.cursor_affinity == UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
            ? UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM) : UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
        return try (requested ?? session.caretGeometry(offset: presentation.cursor_utf8_offset,
            affinity: alternate, in: snapshot.info)).rect.y
    }
}
