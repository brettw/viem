import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVSelectAllTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var text: String?
        var viemGeneration: UInt64 = 1
        var viemIsWritable: Bool { true }
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { text != nil }
        func viemClearContents() -> Int { text = nil; viemGeneration += 1; return Int(viemGeneration) }
        func viemSetString(_ string: String) -> Bool { text = string; viemGeneration += 1; return true }
    }

    private func surface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, Pasteboard) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let pasteboard = Pasteboard()
        surface.pasteboard = pasteboard
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 200, height: 90)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session), pasteboard)
    }

    func testSelectAllCopyCutAndDeleteIncludeTheLastWrappedParagraphForEveryFormatAndLinePolicy() throws {
        let tail = String(repeating: "many wrapping words ", count: 20) + "last العربية 👩‍💻 e\u{301}"
        for (source, type) in [
            ("first\nsecond\n\(tail)", EVDocument.plainTextType),
            ("# first\n\nsecond\n\n\(tail)", EVDocument.markdownType),
            ("# first\n\nsecond\n\n\(tail)", EVDocument.markdownSourceType),
            ("<h1>first</h1><p>second</p><p>\(tail)</p>", EVDocument.htmlType),
            ("<h1>first</h1>\n<p>second</p>\n<p>\(tail)</p>", EVDocument.htmlSourceType),
            ("{\\rtf1 first\\par second\\par \(tail)}", EVDocument.rtfType),
        ] {
            for mode: EVLineMode in [.visual, .physicalSource] {
                if type == EVDocument.rtfType && mode == .physicalSource { continue }
                let (backend, view, session, pasteboard) = try surface(source, type: type)
                try session.setLineMode(mode)
                if type == EVDocument.markdownSourceType || type == EVDocument.htmlSourceType {
                    try session.setParagraphFlow(true)
                }
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 86)
                view.refreshPresentation()
                let text = try backend.formattedText()
                let original = try backend.serializedSource(typeName: type)
                view.perform(menuCommand: .selectAll, sender: nil)
                XCTAssertEqual(view.selectedUTF8Ranges(), [0..<text.utf8.count], "\(type) \(mode)")
                XCTAssertEqual(view.editorView.accessibilitySelectedText(), text)
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
                view.perform(menuCommand: .copy, sender: nil)
                let expectedCopy = (type == EVDocument.markdownSourceType || type == EVDocument.htmlSourceType) ? source : text
                XCTAssertEqual(pasteboard.text, expectedCopy, "\(type) \(mode)")
                XCTAssertEqual(try backend.serializedSource(typeName: type), original)
                view.perform(menuCommand: .selectAll, sender: nil)
                view.perform(menuCommand: .cut, sender: nil)
                XCTAssertEqual(pasteboard.text, expectedCopy)
                XCTAssertEqual(try backend.formattedText(), "")
                XCTAssertNil(view.commandOutput)
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), original)
                view.perform(menuCommand: .selectAll, sender: nil)
                view.perform(menuCommand: .delete, sender: nil)
                XCTAssertEqual(try backend.formattedText(), "")
                XCTAssertNil(view.commandOutput)
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), original)
                view.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.formattedText(), "")
            }
        }
    }

    func testSelectAllDeleteClearsBlockquoteAndListContextBeforeTyping() throws {
        for (source, type) in [
            ("> **quoted**\n> continuation", EVDocument.markdownType),
            ("> - first\n> - second", EVDocument.markdownType),
            ("<!--keep--><blockquote><p><b>quoted</b></p></blockquote>", EVDocument.htmlType),
            ("<blockquote><ul><li>first</li><li>second</li></ul></blockquote>", EVDocument.htmlType),
        ] {
            for mode: EVLineMode in [.visual, .physicalSource] {
                if type == EVDocument.rtfType && mode == .physicalSource { continue }
                let (backend, view, session, _) = try surface(source, type: type)
                try session.setLineMode(mode)
                view.perform(menuCommand: .selectAll, sender: nil)
                view.perform(menuCommand: .delete, sender: nil)
                XCTAssertEqual(try backend.formattedText(), "")
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                let cleared = try backend.serializedSource(typeName: type)
                // Keep deletion and subsequent typing in separate undo groups.
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 105)
                _ = try session.sendText("plain")
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                view.refreshPresentation()
                XCTAssertEqual(try backend.formattedText(), "plain")
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
                XCTAssertNil(view.commandOutput)
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), cleared)
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                view.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), cleared)
            }
        }
    }
}
