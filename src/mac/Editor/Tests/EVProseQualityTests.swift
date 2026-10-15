import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVProseQualityTests: XCTestCase {

  func testRepeatedProseFormattingUndoAndReflowAcrossRichFormats() throws {
    let fixtures = [
      (EVDocument.markdownType, "A **word** and more prose.\nA continuation.\n\nLast paragraph.")
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
          try session.sendTestKeys("gg/word")
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
        try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 105)
        try session.sendKey(kind: UInt32(VIEM_KEY_DOCUMENT_END))
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
