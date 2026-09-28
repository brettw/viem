import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVPendingTypingStyleTests: XCTestCase {
  func testCommandFormattingShortcutsWorkWithoutMenuItemsAndRespectFocus() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      for (key, command) in [("b", EVMenuCommand.bold), ("i", .italic)] {
        let source = Data("word".utf8)
        let backend = EVCoreDocumentBackend()
        try backend.read(source: source, typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        defer { window.close() }
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 4))
        let event = try XCTUnwrap(NSEvent.keyEvent(
          with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0,
          windowNumber: window.windowNumber, context: nil, characters: key,
          charactersIgnoringModifiers: key, isARepeat: false, keyCode: key == "b" ? 11 : 34))
        XCTAssertTrue(surface.editorView.performKeyEquivalent(with: event), type)
        XCTAssertEqual(surface.presentation(for: command).state, .on, type)
        XCTAssertNotEqual(try backend.serializedSource(typeName: type), source, type)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), source, type)

        let field = NSTextField(string: "style name")
        surface.view.addSubview(field)
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertFalse(surface.editorView.performKeyEquivalent(with: event), type)
        XCTAssertEqual(try backend.serializedSource(typeName: type), source, type)
      }
    }
  }

  func testOptionItalicKeepsSourceCleanUntilTypedContentAndUndoIsExact() throws {
    for (type, source) in [
      (EVDocument.markdownType, "word"), (EVDocument.markdownSourceType, "word"),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let session = try XCTUnwrap(surface.session)
      surface.performInput { _ = try session.sendText("i") }
      let revision = try backend.documentState().document_revision
      let event = try XCTUnwrap(
        NSEvent.keyEvent(
          with: .keyDown, location: .zero, modifierFlags: .option, timestamp: 0, windowNumber: 0,
          context: nil, characters: "ˆ", charactersIgnoringModifiers: "i", isARepeat: false,
          keyCode: 34))
      surface.editorView.keyDown(with: event)
      XCTAssertEqual(surface.presentation(for: .italic).state, .on, type)
      XCTAssertEqual(try backend.documentState().document_revision, revision, type)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      let implicit = NSRange(location: NSNotFound, length: 0)
      surface.editorView.insertText("A", replacementRange: implicit)
      surface.editorView.insertText("b", replacementRange: implicit)
      XCTAssertEqual(surface.statusBarState.message, "", type)
      XCTAssertEqual(surface.presentation(for: .italic).state, .on, type)
      XCTAssertNotEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), type)
    }
  }

}
