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

    func testQuotedNamedKeysInsertTheirNamesAndReplaceOnlyOneCharacter() throws {
        for mode in ["i", "R"] {
            let (backend, surface) = try fixture("ABCDE")
            surface.editorView.insertText(mode, replacementRange: noReplacement)
            for (code, shift) in [(UInt16(123), false), (UInt16(51), false), (UInt16(117), false), (UInt16(48), true)] {
                try quote("q", in: surface)
                surface.editorView.keyDown(with: try key(code, shift: shift))
            }
            XCTAssertEqual(try backend.formattedText(), "<Left><BS><Del><S-Tab>" + (mode == "R" ? "E" : "ABCDE"))
            XCTAssertNil(surface.commandOutput)
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

    func testNumericEntryAndQuotedPunctuationBypassTypingAssistance() throws {
        let (backend, surface) = try fixture("")
        let session = try XCTUnwrap(surface.session)
        try session.setSmartQuotes(true)
        surface.editorView.insertText("i", replacementRange: noReplacement)
        for input in ["009", "o101", "x42", "u03b1", "U0001f642", "\""] {
            try quote("v", in: surface)
            surface.editorView.insertText(input, replacementRange: noReplacement)
        }
        XCTAssertEqual(try backend.formattedText(), "\tABα🙂\"")
        XCTAssertNil(surface.commandOutput)
    }

    func testLiteralCommandLineInputDoesNotCompleteSubmitNavigateOrCancel() throws {
        let (backend, surface) = try fixture()
        surface.editorView.insertText(":", replacementRange: noReplacement)
        try quote("v", in: surface)
        surface.editorView.keyDown(with: try key(48))
        try quote("q", in: surface)
        surface.editorView.keyDown(with: try key(36))
        try quote("v", in: surface)
        surface.editorView.keyDown(with: try key(53))
        try quote("q", in: surface)
        surface.editorView.keyDown(with: try key(115))
        try quote("v", in: surface)
        surface.editorView.insertText("u03b1", replacementRange: noReplacement)
        XCTAssertEqual(surface.commandLine?.text, "\t\r\u{1b}<Home>α")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(try backend.formattedText(), "end")
        XCTAssertFalse(backend.persistenceState.isDirty)
        surface.editorView.keyDown(with: try key(53))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }

    func testNormalModeRetainsVisualBlockShortcuts() throws {
        for prefix in ["v", "q"] {
            let (_, surface) = try fixture()
            surface.editorView.keyDown(with: try key(prefix == "v" ? 9 : 12, control: prefix))
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
            XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
        }
    }

    func testNumericTerminatorMovesOnTheUpdatedLayoutAndRecordsOnlyOnce() throws {
        let (backend, surface) = try fixture("one\ntwo\nthree")
        surface.editorView.insertText("qai", replacementRange: noReplacement)
        try quote("v", in: surface)
        surface.editorView.insertText("65", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(125))
        XCTAssertEqual(try backend.formattedText(), "Aone\ntwo\nthree")
        XCTAssertGreaterThanOrEqual(surface.viewPresentation.cursor_utf8_offset, 5)
        XCTAssertLessThanOrEqual(surface.viewPresentation.cursor_utf8_offset, 8)
        surface.editorView.keyDown(with: try key(53))
        surface.editorView.insertText("qgg@a", replacementRange: noReplacement)
        XCTAssertEqual(try backend.formattedText(), "AAone\ntwo\nthree")
        XCTAssertGreaterThanOrEqual(surface.viewPresentation.cursor_utf8_offset, 6)
        XCTAssertLessThan(surface.viewPresentation.cursor_utf8_offset, 9,
                          "The recorded Down key must run once, not move to the third line")
        XCTAssertNil(surface.commandOutput)
    }

    func testNumericCommandLineTerminatorCanStartNormalReplayFromVisualBlock() throws {
        let (backend, surface) = try fixture("one\ntwo")
        surface.editorView.keyDown(with: try key(9, control: "v"))
        surface.editorView.insertText(":normal! i", replacementRange: noReplacement)
        try quote("v", in: surface)
        surface.editorView.insertText("65", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(36))
        XCTAssertEqual(try backend.formattedText(), "Aone\ntwo")
        XCTAssertNil(surface.commandOutput)
    }

    func testVisualBlockKeepsQuotedTabsAndQuotesLiteralBesideSmartQuotes() throws {
        let (backend, surface) = try fixture("one\ntwo")
        try XCTUnwrap(surface.session).setSmartQuotes(true)
        surface.editorView.keyDown(with: try key(9, control: "v"))
        surface.editorView.insertText("jI", replacementRange: noReplacement)
        surface.editorView.insertText("'", replacementRange: noReplacement)
        try quote("v", in: surface)
        surface.editorView.keyDown(with: try key(48))
        try quote("q", in: surface)
        surface.editorView.insertText("\"", replacementRange: noReplacement)
        surface.editorView.insertText("word'", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(53))
        XCTAssertEqual(try backend.formattedText(), "‘\t\"word’one\n‘\t\"word’two")
        XCTAssertNil(surface.commandOutput)
    }
}
