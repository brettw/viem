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
            ("αβ👩‍💻xy", EVDocument.markdownType, "αβ👩‍💻xy", "αβ\n👩‍💻xy"),
            ("# αβ👩‍💻xy", EVDocument.markdownType, "αβ👩‍💻xy", "αβ\n👩‍💻xy"),
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

    func testNativeBackspaceResetsListsAndJoinsQuotesWhileEmptyQuoteReturnResetsStyle() throws {
        for (source, type, at, enter, text) in [
            ("previous\n\n> body\n\nnext", EVDocument.markdownType, 9, false, "previousbody\nnext"),
            ("previous\n\n- body\n\nnext", EVDocument.markdownType, 9, false, "previous\nbody\nnext"),
            ("previous\n\n1. body\n\nnext", EVDocument.markdownType, 9, false, "previous\nbody\nnext"),
            ("previous\n\n> \n\nnext", EVDocument.markdownType, 9, true, "previous\n\nnext"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("i").value)
            view.refreshPresentation()
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
            let previousLength = try backend.formattedText().utf8.count
            view.editorView.keyDown(with: try key(enter ? 36 : 51))
            XCTAssertEqual(try backend.formattedText(), text)
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, UInt64(at + text.utf8.count - previousLength))
            XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            try checkHistory(backend, view, source: source, type: type)
        }
    }

    func testNativeOpenAboveAndBelowUseTheHeadingFollowingStyle() throws {
        for (source, type) in [
            ("# Title", EVDocument.markdownType),
        ] {
            for (command, text) in [("o", "Title\nbody"), ("O", "body\nTitle")] {
                let (backend, view, session) = try surface(source, type: type)
                view.editorView.insertText(command, replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph", "\(type) \(command)")
                view.editorView.insertText("body", replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertEqual(try backend.formattedText(), text)
                XCTAssertEqual(Set(try XCTUnwrap(view.layoutSnapshot).rows.map(\.paragraph_id)).count, 2)
                try checkHistory(backend, view, source: source, type: type)
            }
        }
    }

    func testSourceNativeBackspaceContinuesAfterDeletingTheLastCharacterOfANewParagraph() throws {
        let type = EVDocument.markdownSourceType
        let replacementRange = NSRange(location: NSNotFound, length: 0)
        for selectorRoute in [false, true] {
            for shiftReturn in [false, true] {
                // Keep one pass uninterrupted; checkpoint another before its net-empty edit.
                for historyCheckpoint in [false, true] {
                    let (backend, view, session) = try surface("", type: type)
                    let context = "selector=\(selectorRoute), shift=\(shiftReturn), history=\(historyCheckpoint)"
                    func check(_ source: String, text: String, cursor: UInt64) throws {
                        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), context)
                        XCTAssertEqual(try backend.formattedText(), text, context)
                        XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, cursor, context)
                        XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT), context)
                        XCTAssertNotNil(view.formattedPointInfo(atUTF8Offset: Int(cursor)), context)
                        XCTAssertNil(view.commandOutput, context)
                        let reopened = EVCoreDocumentBackend()
                        try reopened.read(source: Data(source.utf8), typeName: type)
                        XCTAssertEqual(try reopened.serializedSource(typeName: type), Data(source.utf8), context)
                        XCTAssertEqual(try reopened.formattedText(), text, context)
                    }

                    view.editorView.insertText("i", replacementRange: replacementRange)
                    view.editorView.insertText("ab", replacementRange: replacementRange)
                    try check("ab", text: "ab", cursor: 2)
                    view.editorView.keyDown(with: try key(36, shift: shiftReturn))
                    let breakSource = shiftReturn ? "ab\n" : "ab\n\n"
                    try check(breakSource, text: "ab\n", cursor: 3)
                    view.editorView.insertText("c", replacementRange: replacementRange)
                    try check(breakSource + "c", text: "ab\nc", cursor: 4)

                    for (source, text, cursor) in [
                        (breakSource, "ab\n", UInt64(3)), ("ab", "ab", 2),
                        ("a", "a", 1), ("", "", 0), ("", "", 0),
                    ] {
                        if selectorRoute {
                            view.editorView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
                        } else {
                            view.editorView.keyDown(with: try key(51))
                        }
                        try check(source, text: text, cursor: cursor)
                        if historyCheckpoint && source == "ab" {
                            try checkHistory(backend, view, source: "", type: type)
                            try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                            view.refreshPresentation()
                            view.editorView.insertText("A", replacementRange: replacementRange)
                            try check("ab", text: "ab", cursor: 2)
                        }
                    }
                    if historyCheckpoint {
                        try checkHistory(backend, view, source: "ab", type: type)
                    }
                }
            }
        }
    }

}
