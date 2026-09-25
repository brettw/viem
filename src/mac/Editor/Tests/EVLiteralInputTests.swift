import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVLiteralInputTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ source: String = "end") throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-literal-input-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func key(_ code: UInt16, control: String? = nil, shift: Bool = false, characters: String? = nil) throws -> NSEvent {
        var modifiers: NSEvent.ModifierFlags = shift ? [.shift] : []
        if control != nil { modifiers.insert(.control) }
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
            characters: characters ?? control ?? "", charactersIgnoringModifiers: control ?? "",
            isARepeat: false, keyCode: code))
    }

    private func quote(_ prefix: String, in surface: EVEditorSurfaceController) throws {
        surface.editorView.keyDown(with: try key(prefix == "v" ? 9 : 12, control: prefix))
        XCTAssertNotEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
    }

    func testBothPrefixesQuotePhysicalTabReturnEscapeAndControlCharacters() throws {
        for prefix in ["v", "q"] {
            let (backend, surface) = try fixture()
            let view = surface.editorView
            view.insertText("i", replacementRange: noReplacement)
            var expected = ""
            let inputs: [(UInt16, String?, String)] = [
                (48, nil, "\t"), (36, nil, "\r"), (53, nil, "\u{1b}"),
                (8, "c", "\u{03}"), (4, "h", "\u{08}"), (34, "i", "\t"),
                (38, "j", "\0"), (49, " ", "\0"), (33, "[", "\u{1b}"),
                (9, "v", "\u{16}"), (12, "q", "\u{11}")
            ]
            for (code, control, literal) in inputs {
                try quote(prefix, in: surface)
                view.keyDown(with: try key(code, control: control))
                expected += literal
                XCTAssertEqual(try backend.formattedText(), expected + "end")
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
                XCTAssertNil(surface.commandOutput)
            }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data((expected + "end").utf8))
            view.keyDown(with: try key(53))
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.formattedText(), "end")
        }
    }

    func testNativeControlValueIsPreservedWhenThePrintedKeyDiffers() throws {
        let (backend, surface) = try fixture("")
        surface.editorView.insertText("i", replacementRange: noReplacement)
        for (printed, actual) in [("2", "\0"), ("h", "\u{08}"), ("?", "\u{7f}")] {
            try quote("v", in: surface)
            surface.editorView.keyDown(with: try key(18, control: printed, characters: actual))
        }
        XCTAssertEqual(try backend.formattedText(), "\0\u{08}\u{7f}")
    }

    func testNormalModeRetainsVisualBlockShortcuts() throws {
        for prefix in ["v", "q"] {
            let (_, surface) = try fixture()
            surface.editorView.keyDown(with: try key(prefix == "v" ? 9 : 12, control: prefix))
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
            XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
        }
    }

}
