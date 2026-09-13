import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVCommandLineRegisterTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-prompt-register-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data("word rest".utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func control(_ value: String, in view: EVEditorView) throws {
        view.keyDown(with: try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.control], timestamp: 0, windowNumber: 0, context: nil,
            characters: value, charactersIgnoringModifiers: value, isARepeat: false, keyCode: 15)))
    }

    func testNativeRegisterSelectorUsesCoreInsteadOfReplacingPromptText() throws {
        for prompt in [":", "/", "?"] {
            let (backend, surface) = try fixture()
            let view = surface.editorView
            view.insertText("\"ayiw" + prompt, replacementRange: noReplacement)
            try control("r", in: view)
            XCTAssertNotEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING), 0)
            XCTAssertEqual(surface.commandLine?.text, "")
            view.insertText("a", replacementRange: noReplacement)
            XCTAssertEqual(surface.commandLine?.text, "word")
            XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_COMMAND_LINE_REGISTER_PENDING), 0)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
            XCTAssertEqual(try backend.formattedText(), "word rest")
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertNil(surface.commandOutput)
        }
    }

    func testLiteralRegisterVariantAndCancellationKeepPromptEditingActive() throws {
        let (backend, surface) = try fixture()
        let view = surface.editorView
        view.insertText("\"ayiw:", replacementRange: noReplacement)
        try control("r", in: view)
        try control("o", in: view)
        view.insertText("a", replacementRange: noReplacement)
        XCTAssertEqual(surface.commandLine?.text, "word")
        try control("r", in: view)
        try control("c", in: view)
        view.insertText("!", replacementRange: noReplacement)
        XCTAssertEqual(surface.commandLine?.text, "word!")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(try backend.formattedText(), "word rest")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(surface.commandOutput)
    }
}
