import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVLinksTests: XCTestCase {
    func testURLsAreStructuredAndNeverReinterpretedAsCommands() throws {
        let url = try EVLinkOpener.destinationURL("https://example.com/a%20b?q=$(touch%20/tmp/oops);`echo`&x=é", relativeTo: nil)
        XCTAssertEqual(url.host, "example.com")
        XCTAssertTrue(url.absoluteString.contains("$(touch%20/tmp/oops)"))
        XCTAssertFalse(url.absoluteString.contains("%2520"))
        XCTAssertTrue(url.absoluteString.contains("%C3%A9"))
        let relative = try EVLinkOpener.destinationURL("../two words.html#part", relativeTo: URL(fileURLWithPath: "/tmp/docs/one.md"))
        XCTAssertEqual(relative.standardizedFileURL.path, "/tmp/two words.html")
        XCTAssertEqual(relative.fragment, "part")
        for text in ["javascript:alert(1)", "data:text/html,<script/>", "https://example.com/\nnext", "relative", "http:"] {
            XCTAssertThrowsError(try EVLinkOpener.destinationURL(text, relativeTo: nil), text)
        }
    }

    func testContextMenuOpensCurrentLinkAndRejectsStaleMenuWithoutMovingSelection() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("before [label](https://example.com/a?x=1&y=2) after".utf8), typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 800, height: 350)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("7l") }
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.shift], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 109))
        let cursor = surface.viewPresentation.cursor_utf8_offset
        let menu = try XCTUnwrap(surface.editorView.menu(for: event))
        XCTAssertEqual(menu.items.first?.title, "Open link")
        XCTAssertNotNil(menu.items.first?.image)
        XCTAssertTrue(menu.items[1].isSeparatorItem)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        let item = menu.items[0]
        XCTAssertTrue(surface.editorView.validateMenuItem(item))
        var opened: [URL] = []
        surface.editorView.openLinkURL = { url, completion in opened.append(url); completion(nil) }
        surface.editorView.openLink(item)
        XCTAssertEqual(opened.map(\.absoluteString), ["https://example.com/a?x=1&y=2"])
        surface.performInput { _ = try session.sendText("iX") }
        XCTAssertFalse(surface.editorView.validateMenuItem(item))
        surface.editorView.openLink(item)
        XCTAssertEqual(opened.count, 1)
        XCTAssertNotNil(surface.commandOutput)
        withExtendedLifetime(window) {}
    }

    func testPointerMenuOnlyAddsLinkForPaintedAnchorContent() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("<p><a href='https://example.com'>label</a> tail</p>".utf8), typeName: EVDocument.htmlSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 300)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let labelOffset = UInt64("<p><a href='https://example.com'>".utf8.count)
        let cluster = try XCTUnwrap(snapshot.clusters.first { $0.text_start <= labelOffset && labelOffset < $0.text_end })
        let row = try XCTUnwrap(snapshot.rows.first { $0.row_index == cluster.row_index })
        let local = surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(cluster.x + cluster.advance / 2), y: CGFloat(row.y + row.line_advance / 2)))
        func menu(at point: NSPoint) throws -> NSMenu {
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: .rightMouseDown, location: surface.editorView.convert(point, to: nil), modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
            return try XCTUnwrap(surface.editorView.menu(for: event))
        }
        XCTAssertEqual(try menu(at: local).items.first?.title, "Open link")
        XCTAssertEqual(try menu(at: NSPoint(x: 850, y: 250)).items.first?.title, "Cut")
        withExtendedLifetime(window) {}
    }
}
