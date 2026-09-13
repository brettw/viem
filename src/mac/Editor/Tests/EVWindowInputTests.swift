import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVWindowInputTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture() throws -> (EVCoreDocumentBackend, EVDocument, EVDocumentWindowController) {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let backend = EVCoreDocumentBackend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("first line\nsecond line\nthird".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        document.makeWindowControllers()
        let window = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        window.showWindow(nil)
        return (backend, document, window)
    }

    private func surface(_ window: EVDocumentWindowController) throws -> EVEditorSurfaceController {
        try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
    }

    private func control(_ letter: String, in surface: EVEditorSurfaceController, keyCode: UInt16 = 0) throws {
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.control], timestamp: 0, windowNumber: 0, context: nil,
            characters: letter, charactersIgnoringModifiers: letter, isARepeat: false, keyCode: keyCode))
        surface.editorView.keyDown(with: event)
    }

    private func type(_ text: String, in surface: EVEditorSurfaceController) {
        surface.editorView.insertText(text, replacementRange: noReplacement)
    }

    func testCountedSplitAndNewPaneKeepTheirBufferAndSetEditorHeightInRows() throws {
        let (backend, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        type("2", in: original)
        try control("w", in: original)
        type("3", in: original)
        try control("v", in: original)
        XCTAssertEqual(window.paneCount, 2, "CTRL-W CTRL-V must split rather than enter Visual Block")
        let split = try surface(window)
        XCTAssertTrue(split.backend === backend)
        XCTAssertFalse(split === original)
        XCTAssertEqual(split.editorView.bounds.height, 6 * (try XCTUnwrap(split.visualRowHeight)), accuracy: 2)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))

        type("iX", in: split)
        _ = try XCTUnwrap(split.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        split.refreshPresentation()
        XCTAssertTrue(backend.persistenceState.isDirty)
        try control("w", in: split)
        type("5", in: split)
        try control("n", in: split)
        XCTAssertEqual(window.paneCount, 3, "new pane keeps even an unsaved current buffer open")
        let fresh = try surface(window)
        let freshDocument = try XCTUnwrap(window.activeDocument)
        defer { freshDocument.close() }
        XCTAssertFalse(fresh.backend === backend)
        XCTAssertEqual(try fresh.backend.formattedText(), "")
        XCTAssertEqual(try backend.formattedText(), "Xfirst line\nsecond line\nthird")
        XCTAssertEqual(fresh.editorView.bounds.height, 5 * (try XCTUnwrap(fresh.visualRowHeight)), accuracy: 2)
    }

    func testWindowControlAliasesCountsAndCancelSurviveNativeRouting() throws {
        let (_, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        try control("w", in: original)
        type("s", in: original)
        let split = try surface(window)
        for letter in ["h", "l"] {
            try control("w", in: split)
            try control(letter, in: split)
            XCTAssertTrue(window.editorSurface === split)
            XCTAssertNil(split.commandOutput)
        }
        try control("w", in: split)
        let backspace = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 51))
        split.editorView.keyDown(with: backspace)
        XCTAssertTrue(window.editorSurface === split)
        XCTAssertNil(split.commandOutput)

        try control("w", in: split)
        type("4", in: split)
        try control("_", in: split)
        XCTAssertEqual(split.editorView.bounds.height, 4 * (try XCTUnwrap(split.visualRowHeight)), accuracy: 2)
        try control("w", in: split)
        type("2", in: split)
        try control("c", in: split)
        XCTAssertEqual(window.paneCount, 2)
        XCTAssertNil(split.commandOutput)
        try control("w", in: split)
        type("1w", in: split)
        XCTAssertTrue(window.editorSurface === original)
    }

    func testVisualBlockWindowPrefixKeepsSelectionAndColonOpensItsRange() throws {
        let (_, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        try control("v", in: original)
        type("jl", in: original)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        let cursor = original.viewPresentation.cursor_utf8_offset
        try control("w", in: original)
        try control("c", in: original)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        XCTAssertEqual(original.viewPresentation.cursor_utf8_offset, cursor)
        try control("w", in: original)
        type(":", in: original)
        XCTAssertEqual(original.statusBarState.commandLine?.text, "1,2")
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertNil(original.commandOutput)
    }
}
