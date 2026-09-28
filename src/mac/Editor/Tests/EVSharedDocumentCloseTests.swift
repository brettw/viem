import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVSharedDocumentCloseTests: XCTestCase {
    private func fixture() throws -> (EVCoreDocumentBackend, EVDocument, EVDocumentWindowController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("Shared original".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        document.makeWindowControllers()
        let window = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        window.commandDidCloseWindow = {}
        window.showWindow(nil)
        return (backend, document, window)
    }

    private func surface(_ window: EVDocumentWindowController) throws -> EVEditorSurfaceController {
        try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
    }

    private func ex(_ command: String, in surface: EVEditorSurfaceController) {
        surface.editorView.insertText(":" + command, replacementRange: NSRange(location: NSNotFound, length: 0))
        surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    }

    private func edit(_ surface: EVEditorSurfaceController) throws {
        surface.editorView.insertText("iUnsaved ", replacementRange: NSRange(location: NSNotFound, length: 0))
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
    }

    func testExQuitOfDirtySharedPaneSkipsReviewAndKeepsTheFinalViewProtected() throws {
        let (backend, document, window) = try fixture()
        defer { document.close() }
        var reviews = 0
        document.closeReviewDecisionHandler = { reply in reviews += 1; reply(false) }
        let original = try surface(window)
        ex("split", in: original)
        let split = try surface(window)
        XCTAssertFalse(split === original)
        try edit(split)
        let unsaved = try backend.serializedSource(typeName: EVDocument.plainTextType)

        ex("q", in: split)
        XCTAssertEqual(window.paneCount, 1)
        XCTAssertTrue(window.editorSurface === original)
        XCTAssertEqual(reviews, 0)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), unsaved)
        XCTAssertTrue(window.window?.isVisible == true)

        // Final-view Ex quit retains its existing E37 behavior; native close
        // offers Save/Don't Save/Cancel and must review exactly once.
        ex("q", in: original)
        XCTAssertEqual(window.paneCount, 1)
        XCTAssertTrue(window.window?.isVisible == true)
        window.window?.performClose(nil)
        XCTAssertEqual(reviews, 1)
        XCTAssertTrue(window.window?.isVisible == true)
        XCTAssertTrue(backend.persistenceState.isDirty)
    }

    func testExQuitOfDirtySharedWindowKeepsTheOtherWindowAndItsEdits() throws {
        let (backend, document, first) = try fixture()
        defer { document.close() }
        document.showAdditionalWindow()
        let second = try XCTUnwrap(document.windowControllers.last as? EVDocumentWindowController)
        second.commandDidCloseWindow = {}
        var reviews = 0
        document.closeReviewDecisionHandler = { reply in reviews += 1; reply(false) }
        let original = try surface(first)
        try edit(original)
        let unsaved = try backend.serializedSource(typeName: EVDocument.plainTextType)

        ex("q", in: original)
        XCTAssertEqual(reviews, 0)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertTrue(second.window?.isVisible == true)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), unsaved)
        second.window?.performClose(nil)
        XCTAssertEqual(reviews, 1)
        XCTAssertTrue(second.window?.isVisible == true)
    }
}
