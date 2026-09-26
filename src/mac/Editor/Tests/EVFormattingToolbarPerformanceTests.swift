import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVFormattingToolbarPerformanceTests: XCTestCase {
  func testLargeDocumentToolbarRefreshAndFormattingToggles() throws {
    let markdown = "# Heading\n\n- First item\n- Second item\n\n" + String(repeating: "A paragraph with **bold** and `code` text.\n\n", count: 4000)
    let html = "<h1>Heading</h1><ul><li>First item</li><li>Second item</li></ul>" + String(repeating: "<p>A paragraph with <b>bold</b> and <code>code</code> text.</p>", count: 4000)
    let rtf = "{\\rtf1 First item\\par Second item\\par " + String(repeating: "A paragraph with {\\b bold} text.\\par ", count: 4000) + "}"
    for (format, source) in [(EVDocument.markdownType, markdown), (EVDocument.markdownSourceType, markdown),
                             (EVDocument.htmlType, html), (EVDocument.htmlSourceType, html), (EVDocument.rtfType, rtf)] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: format)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 350)
      surface.viewDidLayout()
      let session = try XCTUnwrap(surface.session)
      let toolbar = surface.formattingToolbar
      let text = try backend.formattedText() as NSString
      let at = text.range(of: "Second item").location
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
      toolbar.refresh()
      let items = toolbar.paragraphStyle.itemArray
      var elapsed = 0.0
      for offset in 0..<10 {
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at + offset, length: 0))
        let start = CFAbsoluteTimeGetCurrent()
        toolbar.refresh()
        elapsed += CFAbsoluteTimeGetCurrent() - start
        XCTAssertEqual(toolbar.paragraphStyle.itemArray.map(ObjectIdentifier.init), items.map(ObjectIdentifier.init))
      }
      print("TOOLBAR_PROFILE \(format) average refresh: \(elapsed / 10 * 1000)ms")
      // A broad limit catches the former 0.4–1.5s full-document preparation;
      // structural core tests separately assert bounded work, without a clock.
      XCTAssertLessThan(elapsed / 10, 0.050)
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 6))
      var start = CFAbsoluteTimeGetCurrent()
      toolbar.refresh()
      print("TOOLBAR_PROFILE \(format) selection refresh: \((CFAbsoluteTimeGetCurrent() - start) * 1000)ms")
      let bold = try XCTUnwrap(toolbar.commandButtons[.bold])
      start = CFAbsoluteTimeGetCurrent()
      toolbar.performCommand(bold)
      let selectedElapsed = CFAbsoluteTimeGetCurrent() - start
      print("TOOLBAR_PROFILE \(format) selected bold action: \(selectedElapsed * 1000)ms")
      // HTML Source intentionally uses one huge unwrapped physical line here;
      // its complete line measurement remains separate from toolbar refresh.
      if format != EVDocument.htmlSourceType { XCTAssertLessThan(selectedElapsed, 0.250) }
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(bold.state, .on)
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
      surface.performInput { _ = try session.sendText("i") }
      toolbar.refresh()
      let revision = try backend.documentState().document_revision
      let italic = try XCTUnwrap(toolbar.commandButtons[.italic])
      start = CFAbsoluteTimeGetCurrent()
      toolbar.performCommand(italic)
      let typingElapsed = CFAbsoluteTimeGetCurrent() - start
      print("TOOLBAR_PROFILE \(format) typing italic action: \(typingElapsed * 1000)ms")
      XCTAssertLessThan(typingElapsed, 0.100)
      XCTAssertEqual(try backend.documentState().document_revision, revision)
      XCTAssertEqual(italic.state, .on)
    }
  }
}
