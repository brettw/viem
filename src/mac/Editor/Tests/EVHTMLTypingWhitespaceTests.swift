import AppKit
import CEvimCore
import EvimAppShell
import XCTest

@testable import EvimEditor

final class EVHTMLTypingWhitespaceTests: XCTestCase {
  @MainActor
  func testDeletionAndParagraphBreakProtectSpacesAndKeepTypingCaret() throws {
    for splitParagraph in [false, true] {
      let original = Data("<p>A B</p><!--keep-->".utf8)
      let backend = EVCoreDocumentBackend()
      try backend.read(source: original, typeName: EVDocument.htmlType)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let session = try XCTUnwrap(surface.session)
      if splitParagraph {
        try session.sendText("l")
        try session.sendText("i")
        try session.sendKey(kind: UInt32(EVIM_KEY_ENTER))
      } else {
        try session.sendText("A")
        try session.sendKey(kind: UInt32(EVIM_KEY_BACKSPACE))
      }
      let protectedSource = splitParagraph
        ? "<p>A</p><p>&nbsp;B</p><!--keep-->" : "<p>A&nbsp;</p><!--keep-->"
      XCTAssertEqual(
        try backend.serializedSource(typeName: EVDocument.htmlType), Data(protectedSource.utf8))

      surface.editorView.insertText(
        "C", replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertEqual(surface.statusBarState.message, "")
      let expected = Data((splitParagraph
        ? "<p>A</p><p>C B</p><!--keep-->" : "<p>A C</p><!--keep-->").utf8)
      let expectedText = splitParagraph ? "A\nC B" : "A C"
      XCTAssertEqual(try backend.formattedText(), expectedText)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)
      let reopened = EVCoreDocumentBackend()
      try reopened.read(source: expected, typeName: EVDocument.htmlType)
      XCTAssertEqual(try reopened.formattedText(), expectedText)

      try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), original)
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)
    }
  }

  @MainActor
  func testTypingOrdinaryWordSpaceProducesPlainHTMLAndRoundTripsUndo() throws {
    let original = Data("<p></p>".utf8)
    let backend = EVCoreDocumentBackend()
    try backend.read(source: original, typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    try session.sendText("i")

    let text = "Hello, world!"
    for scalar in text.unicodeScalars {
      surface.editorView.insertText(
        String(scalar), replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertEqual(surface.statusBarState.message, "")
      if scalar == " " {
        XCTAssertEqual(try backend.formattedText(), "Hello,\u{00a0}")
        XCTAssertEqual(
          try backend.serializedSource(typeName: EVDocument.htmlType),
          Data("<p>Hello,&nbsp;</p>".utf8))
      }
    }

    let expected = Data("<p>Hello, world!</p>".utf8)
    XCTAssertEqual(try backend.formattedText(), text)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)

    let reopened = EVCoreDocumentBackend()
    try reopened.read(source: expected, typeName: EVDocument.htmlType)
    XCTAssertEqual(try reopened.formattedText(), text)

    try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), original)
    XCTAssertEqual(try backend.formattedText(), "")
    surface.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)
    XCTAssertEqual(try backend.formattedText(), text)
  }

  @MainActor
  func testLeadingTypedSpaceRemainsEditableAndUsesNbspWithoutStyleDefinitions() throws {
    let original = Data("<p></p><!--keep-->".utf8)
    let backend = EVCoreDocumentBackend()
    try backend.read(source: original, typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    try session.sendText("i")

    for scalar in " hello ".unicodeScalars {
      surface.editorView.insertText(
        String(scalar), replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertEqual(surface.statusBarState.message, "")
    }
    let expected = Data("<p>&nbsp;hello&nbsp;</p><!--keep-->".utf8)
    let expectedText = "\u{00a0}hello\u{00a0}"
    XCTAssertEqual(try backend.formattedText(), expectedText)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)

    let reopened = EVCoreDocumentBackend()
    try reopened.read(source: expected, typeName: EVDocument.htmlType)
    XCTAssertEqual(try reopened.formattedText(), expectedText)

    try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), original)
    surface.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), expected)
    XCTAssertEqual(try backend.formattedText(), expectedText)
  }
}
