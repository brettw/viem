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

    func testLinkRoutesSeparateHeadingsDocumentsAndWebDestinations() throws {
        XCTAssertEqual(try EVLinkOpener.destination("#caf%C3%A9-heading", relativeTo: nil), .fragment("café-heading"))
        XCTAssertEqual(try EVLinkOpener.destination("#", relativeTo: nil), .fragment(""))
        XCTAssertEqual(try EVLinkOpener.destination("../two%20words.md#part%202", relativeTo: URL(fileURLWithPath: "/tmp/docs/one.md")),
                       .document(URL(fileURLWithPath: "/tmp/two words.md"), fragment: "part 2"))
        XCTAssertEqual(try EVLinkOpener.destination("/tmp/two words.md", relativeTo: nil),
                       .document(URL(fileURLWithPath: "/tmp/two words.md"), fragment: nil))
        XCTAssertEqual(try EVLinkOpener.destination("https://example.com/a%20b#part", relativeTo: nil),
                       .web(URL(string: "https://example.com/a%20b#part")!))
        for destination in ["javascript:alert(1)", "data:text/plain,example", "#bad\nheading", "#bad%0Aheading", "/tmp/bad%0Afile.md", ""] {
            XCTAssertThrowsError(try EVLinkOpener.destination(destination, relativeTo: nil))
        }
        var externalError: Error?
        EVLinkOpener.open(URL(fileURLWithPath: "/tmp/example.md")) { externalError = $0 }
        XCTAssertNotNil(externalError, "Local documents must never be routed through the browser opener")
    }

    func testHeadingLinkNavigatesCurrentDocumentWithoutChangingSourceAndRejectsStaleTarget() throws {
        let source = "[Jump](#second-heading)\n\n# First heading\n\n# Second heading\n\nbody"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 800, height: 350)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        let initial = surface.viewPresentation
        let target = EVLinkMenuTarget(documentID: initial.document_id, revision: initial.document_revision, offset: 1)
        var browserOpens = 0
        surface.editorView.openLinkURL = { _, completed in browserOpens += 1; completed(nil) }
        XCTAssertEqual(try backend.linkDestination(at: target), "#second-heading")
        surface.openLink(at: target)
        XCTAssertNil(surface.commandOutput, surface.statusBarState.message)
        let presentedText = try backend.formattedText()
        let heading = try XCTUnwrap(presentedText.range(of: "Second heading"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(presentedText[..<heading.lowerBound].utf8.count))
        XCTAssertEqual(browserOpens, 0)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
        surface.performInput { _ = try session.sendTestKeys("iX") }
        let afterEdit = surface.viewPresentation.cursor_utf8_offset
        surface.openLink(at: target)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, afterEdit)
        XCTAssertNotNil(surface.commandOutput)
        withExtendedLifetime(window) {}
    }

    func testLineLinkUsesPhysicalSourceLineAndCenters() throws {
        let source = "[Jump](#L83)\n\n" + (1...80).map { "**line \($0)**" }.joined(separator: "\n\n")
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 800, height: 350)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { try session.setViewMargins(.init(top: 17, left: 10, bottom: 29, right: 10)) }
        let state = surface.viewPresentation
        surface.openLink(at: EVLinkMenuTarget(documentID: state.document_id, revision: state.document_revision, offset: 1))
        XCTAssertNil(surface.commandOutput, surface.statusBarState.message)
        let text = try backend.formattedText()
        let line = try XCTUnwrap(text.range(of: "line 41"))
        let offset = UInt64(text[..<line.lowerBound].utf8.count)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, offset)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let row = try XCTUnwrap(snapshot.rows.first { $0.text_start == offset })
        let rowTop = row.y
        let bottom = row.y + row.ascent + row.descent + row.leading
        XCTAssertEqual((rowTop + bottom) / 2 - surface.viewportState.top,
            (snapshot.info.viewport_height + 17 - 29) / 2, accuracy: 1)
        let cursor = surface.viewPresentation.cursor_utf8_offset
        let top = surface.viewportState.top
        XCTAssertThrowsError(try surface.navigateToLinkFragment("L999"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(surface.viewportState.top, top)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        withExtendedLifetime(window) {}
    }

    func testContextMenuOpensCurrentLinkAndRejectsStaleMenuWithoutMovingSelection() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("before [label](https://example.com/a?x=1&y=2) after".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 800, height: 350)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendTestKeys("7l") }
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
        surface.performInput { _ = try session.sendTestKeys("iX") }
        XCTAssertFalse(surface.editorView.validateMenuItem(item))
        surface.editorView.openLink(item)
        XCTAssertEqual(opened.count, 1)
        XCTAssertNotNil(surface.commandOutput)
        withExtendedLifetime(window) {}
    }

    func testPointerMenuOnlyAddsLinkForPaintedAnchorContent() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("[label](https://example.com) tail".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 300)
        let window = NSWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let labelOffset = UInt64(1)
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
