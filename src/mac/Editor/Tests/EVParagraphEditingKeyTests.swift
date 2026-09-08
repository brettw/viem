import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVParagraphEditingKeyTests: XCTestCase {
    private func surface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        view.viewDidLayout()
        return (backend, view, try XCTUnwrap(view.session))
    }

    private func key(_ code: UInt16, shift: Bool = false) throws -> NSEvent {
        try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: shift ? [.shift] : [], timestamp: 0, windowNumber: 0, context: nil,
            characters: code == 51 ? "\u{7f}" : "\r", charactersIgnoringModifiers: "\r",
            isARepeat: false, keyCode: code))
    }

    private func checkHistory(_ backend: EVCoreDocumentBackend, _ view: EVEditorSurfaceController,
                              source: String, type: String) throws {
        let saved = try backend.serializedSource(typeName: type)
        let text = try backend.formattedText()
        let reopened = EVCoreDocumentBackend()
        try reopened.read(source: saved, typeName: type)
        XCTAssertEqual(try reopened.formattedText(), text)
        XCTAssertNotNil(view.formattedPointInfo(atUTF8Offset: Int(view.viewPresentation.cursor_utf8_offset)))
        XCTAssertNil(view.commandOutput)
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        view.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertNil(view.commandOutput)
    }

    func testPhysicalShiftReturnAndLineBreakSelectorKeepOneParagraphWhileReturnCreatesTwo() throws {
        for (source, type, before, after) in [
            ("<p>αβ👩‍💻xy</p>", EVDocument.htmlType, "αβ👩‍💻xy", "αβ\n👩‍💻xy"),
            ("αβ👩‍💻xy", EVDocument.markdownType, "αβ👩‍💻xy", "αβ\n👩‍💻xy"),
            ("# αβ👩‍💻xy", EVDocument.markdownType, "αβ👩‍💻xy", "αβ\n👩‍💻xy"),
            ("{\\rtf1 abcd}", EVDocument.rtfType, "abcd", "ab\ncd"),
        ] {
            for route in 0..<4 {
                let (backend, view, session) = try surface(source, type: type)
                XCTAssertEqual(try backend.formattedText(), before)
                try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("i").value)
                try session.sendKey(kind: UInt32(VIEM_KEY_RIGHT))
                try session.sendKey(kind: UInt32(VIEM_KEY_RIGHT))
                view.refreshPresentation()
                let style = try session.selectedNamedStyles().paragraph
                if route == 2 {
                    view.editorView.doCommand(by: #selector(NSResponder.insertLineBreak(_:)))
                } else {
                    view.editorView.keyDown(with: try key(route == 1 ? 76 : 36, shift: route != 3))
                }
                XCTAssertEqual(try backend.formattedText(), after)
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                if route != 3 {
                    XCTAssertEqual(try session.selectedNamedStyles().paragraph, style)
                }
                let rows = try XCTUnwrap(view.layoutSnapshot).rows
                XCTAssertEqual(Set(rows.map(\.paragraph_id)).count, route == 3 ? 2 : 1)
                XCTAssertEqual(Set(rows.map(\.hard_line_index)).count, 2)
                try checkHistory(backend, view, source: source, type: type)
            }
        }
    }

    func testNativeBackspaceAndEmptyQuoteReturnRemoveOnlyCurrentParagraphStyle() throws {
        for (source, type, at, enter, text) in [
            ("<p>previous</p><blockquote><p>body</p></blockquote><p>next</p>", EVDocument.htmlType, 9, false, "previous\nbody\nnext"),
            ("<p>previous</p><ul><li>body</li></ul><p>next</p>", EVDocument.htmlType, 9, false, "previous\nbody\nnext"),
            ("<p>previous</p><ol><li>body</li></ol><p>next</p>", EVDocument.htmlType, 9, false, "previous\nbody\nnext"),
            ("previous\n\n> body\n\nnext", EVDocument.markdownType, 9, false, "previous\nbody\nnext"),
            ("previous\n\n- body\n\nnext", EVDocument.markdownType, 9, false, "previous\nbody\nnext"),
            ("previous\n\n1. body\n\nnext", EVDocument.markdownType, 9, false, "previous\nbody\nnext"),
            ("<p>previous</p><blockquote></blockquote><p>next</p>", EVDocument.htmlType, 9, true, "previous\n\nnext"),
            ("previous\n\n> \n\nnext", EVDocument.markdownType, 9, true, "previous\n\nnext"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("i").value)
            view.refreshPresentation()
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
            view.editorView.keyDown(with: try key(enter ? 36 : 51))
            XCTAssertEqual(try backend.formattedText(), text)
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, UInt64(at))
            XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            try checkHistory(backend, view, source: source, type: type)
        }
    }
}
