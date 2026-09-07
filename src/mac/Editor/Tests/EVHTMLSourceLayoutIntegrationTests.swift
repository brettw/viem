import AppKit
import CEvimCore
import EvimAppShell
import XCTest
@testable import EvimEditor

@MainActor
final class EVHTMLSourceLayoutIntegrationTests: XCTestCase {
    func testSourceSwitchAfterEditingPreformattedBlankLinesKeepsLayoutAndHistory() throws {
        let source = """
        <p>This first HTML paragraph has a source newline
        that flows into one paragraph.</p>
        <p>This second paragraph has a readable vertical gap.</p>
        <pre>    first_code_line(&amp;value)
            second_code_line()

            fourth_code_line()</pre>
        <ol start="9"><li>The first numbered item has enough text to wrap across several visual rows when magnified.
        This source continuation should align with the item text.</li><li><div>The next item displays ten and keeps its list formatting inside a div.</div></li></ol>
        <p>A final paragraph.</p>
        """
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 1100, height: 680)
        _ = try session.setScale(1.75)
        for ch in "/first_code".unicodeScalars {
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: ch.value)
        }
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_ENTER))
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: 65)
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_ENTER))
        for ch in "    added_line()" { _ = try session.sendText(String(ch)) }
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
        let edited = try backend.serializedSource(typeName: EVDocument.htmlType)
        XCTAssertNotEqual(edited, Data(source.utf8))
        XCTAssertTrue(String(decoding: edited, as: UTF8.self).contains("<br>    added_line()"))
        XCTAssertFalse(String(decoding: edited, as: UTF8.self).contains("white-space:"))

        _ = try session.setFormat(.htmlSource, expected: backend.documentState())
        for width in [900, 320, 1100] {
            _ = try session.resize(width: CGFloat(width), height: 680)
            let layout = try session.layoutExport()
            XCTAssertFalse(layout.rows.isEmpty)
            XCTAssertFalse(layout.clusters.isEmpty)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), edited)
        }
        surface.refreshPresentation()
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(surface.formattedText, String(decoding: edited, as: UTF8.self))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(backend.sourceFormat, .html)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), edited)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
    }
}
