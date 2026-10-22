import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBlockExitTests: XCTestCase {
    private func makeSurface(_ source: String) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-block-exit-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .paragraphFirstLineIndent, .float(0)),
            (.baseParagraph, .paragraphLeadingIndent, .float(0)),
            (EVStyleTestFixtures.block("BulletedList1"), .blockPaddingLeft, .float(36)),
            (EVStyleTestFixtures.block("Code Block"), .blockPaddingLeft, .float(24)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func key(_ code: UInt16 = 36, modifiers: NSEvent.ModifierFlags = []) throws -> NSEvent {
        let character = code == 51 ? "\u{7f}" : code == 76 ? "\u{3}" : "\r"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
            characters: character, charactersIgnoringModifiers: character,
            isARepeat: false, keyCode: code))
    }

    private func insertAtEnd(_ backend: EVCoreDocumentBackend, _ surface: EVEditorSurfaceController,
                             _ session: EVCoreViewSession) throws {
        surface.performInput { _ = try session.sendText("i") }
        let text = try backend.formattedText()
        // The end of a final nonempty paragraph is an upstream boundary;
        // AX's downstream-only placement can decline that endpoint. Use the
        // native document-end command to reach the intended insertion point.
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfDocument(_:)))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(text.utf8.count))
    }

    private func caretRect(_ surface: EVEditorSurfaceController) throws -> NSRect {
        // Inspect the snapshot published by the native input path immediately,
        // before typing or an explicit refresh can repair stale indentation.
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        return try XCTUnwrap(surface.editorView.caretItemGeometry(snapshot).rect)
    }

    private func proseX(_ surface: EVEditorSurfaceController) throws -> CGFloat {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        return try XCTUnwrap(surface.editorView.caretItemGeometry(snapshot,
            target: .boundary(offset: 0, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM))).rect).minX
    }

    private func assertHistory(_ backend: EVCoreDocumentBackend, _ surface: EVEditorSurfaceController,
                               original: String) throws {
        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        let text = try backend.formattedText()
        let (reopened, _, _) = try makeSurface(String(decoding: saved, as: UTF8.self))
        XCTAssertEqual(try reopened.formattedText(), text)
        surface.editorView.cancelOperation(nil)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(original.utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertNil(surface.commandOutput)
    }

    func testReturnAndBackspaceImmediatelyUnindentTheCaretAfterAnEmptyBullet() throws {
        for exitKey in [UInt16(36), 76, 51] {
            let source = "Before\n\n- Item"
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            surface.editorView.keyDown(with: try key())
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(try session.layoutExport().decorations.count, 2)
            XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface))

            surface.editorView.keyDown(with: try key(exitKey))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            XCTAssertEqual(try caretRect(surface).minX, try proseX(surface), accuracy: 0.01,
                "exiting with key \(exitKey) must move the empty caret before any text is typed")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            let emptyStart = surface.viewPresentation.cursor_utf8_offset
            surface.editorView.insertText("After", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\nAfter")
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let firstTyped = try XCTUnwrap(snapshot.clusters.first { $0.text_start == emptyStart })
            XCTAssertEqual(surface.editorView.viewRect(firstTyped.typographic_bounds).minX,
                try proseX(surface), accuracy: 0.01)
            try assertHistory(backend, surface, original: source)
        }
    }

    func testOptionReturnKeepsTheEmptyContinuationParagraphIndentedInsideItsListItem() throws {
        for returnKey in [UInt16(36), 76] {
            let source = "Before\n\n- Item"
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            surface.editorView.keyDown(with: try key(returnKey, modifiers: [.option]))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface),
                "Option-Return creates a paragraph that still belongs to the list item")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            surface.editorView.insertText("Continuation", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\nContinuation")
            XCTAssertEqual(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self),
                "Before\n\n- Item\n\n  Continuation<span></span>")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            try assertHistory(backend, surface, original: source)
        }
    }

    func testThirdReturnExitsTheFinalCodeBlockWithAnImmediatelyUnindentedCaret() throws {
        for source in ["Before\n\n```rust\nbody\n```", "Before\n\n```rust\nbody"] {
            for returnKey in [UInt16(36), 76] {
                let (backend, surface, session) = try makeSurface(source)
                try insertAtEnd(backend, surface, session)
                for count in 1...2 {
                    surface.editorView.keyDown(with: try key(returnKey))
                    XCTAssertEqual(try backend.formattedText(), "Before\nbody" + String(repeating: "\n", count: count))
                    XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block",
                        "the first two Returns must keep their blank rows inside code")
                    XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface))
                }
                surface.editorView.keyDown(with: try key(returnKey))
                XCTAssertEqual(try backend.formattedText(), "Before\nbody\n")
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
                XCTAssertEqual(try caretRect(surface).minX, try proseX(surface), accuracy: 0.01)
                surface.editorView.insertText("After", replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertEqual(try backend.formattedText(), "Before\nbody\nAfter")
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
                try assertHistory(backend, surface, original: source)
            }
        }
    }

    func testTwoReturnsAllowTypingAfterABlankCodeLine() throws {
        let source = "Before\n\n```rust\nbody\n```"
        let (backend, surface, session) = try makeSurface(source)
        try insertAtEnd(backend, surface, session)
        surface.editorView.keyDown(with: try key())
        surface.editorView.keyDown(with: try key())
        surface.editorView.insertText("next", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(try backend.formattedText(), "Before\nbody\n\nnext")
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("Before\n\n```rust\nbody\n\nnext\n```".utf8))
        try assertHistory(backend, surface, original: source)
    }
}
