import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVProseQualityTests: XCTestCase {

  func testCommonEditingCommandsDoNotPoisonReflowOrUndo() throws {
    let fixtures = [
      (EVDocument.plainTextType, "First words with café and emoji 👩🏽‍💻.\nSecond paragraph.\nTail."),
      (
        EVDocument.markdownType,
        "**First** words with café and emoji 👩🏽‍💻.\n\nSecond paragraph.\n\nTail."
      ),
      (
        EVDocument.rtfType,
        #"{\rtf1 {\b First} words with caf\u233? and more.\par Second paragraph.\par Tail.}"#
      ),
    ]
    for (type, source) in fixtures {
      for command in ["gg0dw", "gg0ciw", "G0o", "gg0viw~", "ggyyGp"] {
        check("\(type), \(command)") {
          let backend = EVCoreDocumentBackend()
          try backend.read(source: Data(source.utf8), typeName: type)
          let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
          surface.loadViewIfNeeded()
          let session = try XCTUnwrap(surface.session)
          try session.resize(width: 170, height: 100)
          try session.sendText(command)
          if ["gg0ciw", "G0o"].contains(command) {
            try session.sendText("Changed é👩🏽‍💻")
            try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
          }
          for width in [1, 120, 800, 240] {
            try session.resize(width: CGFloat(width), height: 160)
            surface.refreshPresentation()
            XCTAssertNil(
              surface.commandOutput,
              "\(type), \(command), width \(width): \(surface.commandOutput ?? "")")
            XCTAssertEqual(
              try session.layoutExport().info.identity.document_revision,
              backend.currentDocumentState.document_revision)
          }
          XCTAssertNotEqual(
            try backend.serializedSource(typeName: type), Data(source.utf8), "\(type), \(command)")
          try session.undo()
          XCTAssertEqual(
            try backend.serializedSource(typeName: type), Data(source.utf8), "\(type), \(command)")
        }
      }
    }
  }

  func testRepeatedProseFormattingUndoAndReflowAcrossRichFormats() throws {
    let fixtures = [
      (EVDocument.markdownType, "A **word** and more prose.\nA continuation.\n\nLast paragraph."),
      (
        EVDocument.markdownSourceType,
        "A **word** and more prose.\nA continuation.\n\nLast paragraph."
      ),
      (EVDocument.rtfType, #"{\rtf1 A {\b word} and more prose.\par Last paragraph.}"#),
    ]
    for (type, source) in fixtures {
      var phase = "open"
      check("\(type), \(phase)") {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let peer = try EVCoreViewSession(document: backend, width: 220, height: 180)
        let originalText = try backend.formattedText()
        for iteration in 0..<12 {
          phase = "iteration \(iteration), search"
          try session.sendText("gg/word")
          try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
          try session.sendText("i")
          surface.refreshPresentation()
          surface.perform(menuCommand: .italic, sender: nil)
          phase = "iteration \(iteration), italic input"
          try session.sendText("é👩🏽‍💻 ")
          surface.refreshPresentation()
          surface.perform(menuCommand: .italic, sender: nil)
          phase = "iteration \(iteration), plain input"
          try session.sendText("plain ")
          try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
          phase = "iteration \(iteration), reflow"
          try session.setWrap(iteration % 2 == 0)
          try session.setScale([0.75, 1, 2.5][iteration % 3])
          try session.resize(width: CGFloat([120, 600, 275][iteration % 3]), height: 180)
          _ = session.provider.invalidateMetrics()
          surface.refreshPresentation()
          XCTAssertNil(
            surface.commandOutput, "\(type), iteration \(iteration): \(surface.commandOutput ?? "")"
          )
          XCTAssertTrue(try backend.formattedText().contains("é👩🏽‍💻 "), type)
          try peer.refreshLayoutIfNeeded()
          XCTAssertEqual(
            try peer.layoutExport().info.identity.document_revision,
            backend.currentDocumentState.document_revision)
          phase = "iteration \(iteration), undo/redo"
          let edited = try backend.serializedSource(typeName: type)
          try session.undo()
          XCTAssertEqual(
            try backend.serializedSource(typeName: type), Data(source.utf8),
            "\(type), iteration \(iteration)")
          XCTAssertEqual(try backend.formattedText(), originalText, type)
          try session.redo()
          XCTAssertEqual(try backend.serializedSource(typeName: type), edited, type)
          try session.undo()
          surface.refreshPresentation()
          XCTAssertNil(surface.commandOutput, "\(type), iteration \(iteration)")
        }
      }
    }
  }

  func testEndOfMarkdownRepeatedReturnsRemainEditable() throws {
    for source in ["", "Word", "**Word**", "First line\ncontinued prose"] {
      check(source) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let session = try EVCoreViewSession(document: backend, width: 200, height: 120)
        let original = try backend.formattedText()
        try session.sendText("GA")
        for count in 1...4 {
          try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
          XCTAssertEqual(
            try backend.formattedText(), original + String(repeating: "\n", count: count))
        }
        try session.sendText("Tail")
        XCTAssertEqual(try backend.formattedText(), original + "\n\n\n\nTail")
        try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        try session.undo()
        XCTAssertEqual(
          try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
      }
    }
  }

  private func check(
    _ label: @autoclosure () -> String, file: StaticString = #filePath, line: UInt = #line,
    _ body: () throws -> Void
  ) {
    do { try body() } catch { XCTFail("\(label()): \(error)", file: file, line: line) }
  }
}
