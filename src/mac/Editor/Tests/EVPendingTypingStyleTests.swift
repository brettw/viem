import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVPendingTypingStyleTests: XCTestCase {
  func testOptionItalicKeepsSourceCleanUntilTypedContentAndUndoIsExact() throws {
    for (type, source) in [
      (EVDocument.markdownType, "word"), (EVDocument.markdownSourceType, "word"),
      (EVDocument.rtfType, #"{\rtf1 word{\*\opaque keep}}"#),
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

  func testRTFBoldAndParagraphStyleMenuApplyAtCurrentParagraph() throws {
    let source = #"{\rtf1 First\par Second{\*\opaque keep}}"#
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: EVDocument.rtfType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    surface.performInput { _ = try session.sendText("Gi") }
    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    let action = EVStyleMenuAction(
      kind: .assign, role: .paragraph, stableID: "Heading2", documentID: catalogue.documentID,
      documentRevision: catalogue.documentRevision, styleSheetRevision: catalogue.styleSheetRevision
    )
    surface.perform(styleMenuAction: action, sender: nil)
    XCTAssertEqual(surface.statusBarState.message, "")
    XCTAssertEqual(surface.formattedText, "First\nSecond")
    let named = try session.selectedNamedStyles()
    let sheet = try backend.styleSheetSnapshot()
    let assigned = try XCTUnwrap(
      named.paragraph.flatMap { sheet.definition(namespace: .block, id: $0) })
    XCTAssertEqual(assigned.name, "Heading 2")
    surface.perform(menuCommand: .bold, sender: nil)
    XCTAssertEqual(surface.presentation(for: .bold).state, .on)
    surface.editorView.insertText(
      "Added", replacementRange: NSRange(location: NSNotFound, length: 0))
    XCTAssertEqual(surface.statusBarState.message, "")
    XCTAssertTrue(surface.formattedText.contains("Added"))
  }
}
