import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVCodeBlockLanguageTests: XCTestCase {
    private final class MenuTracker: NSObject, NSMenuDelegate {
        var menu: NSMenu?
        var highlighted: [String] = []
        func menu(_ menu: NSMenu, willHighlight item: NSMenuItem?) {
            if let item { highlighted.append(item.title) }
        }
    }

    private func trackLanguagePicker(_ surface: EVEditorSurfaceController, keys: [(UInt16, String)], enabled: Bool = true) throws {
        let window = try XCTUnwrap(surface.editorView.window)
        window.contentView?.layoutSubtreeIfNeeded()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let rect = surface.editorView.viewRect(label.typographic_bounds)
        let location = surface.editorView.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil)
        let tracker = MenuTracker()
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main) { note in
            MainActor.assumeIsolated {
                if let menu = note.object as? NSMenu, menu.title == "Code block language" {
                    tracker.menu = menu
                    menu.delegate = tracker
                }
            }
        }
        defer { NotificationCenter.default.removeObserver(observer) }
        let events = try keys.reversed().map { keyCode, characters in
            try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: location, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
        }
        // NSMenu.popUp starts standalone tracking; unlike an NSComboBox button,
        // it has no native mouse-down to release before keyboard selection.
        // Deliver keys after AppKit enters tracking, so popup setup cannot
        // discard events queued before the standalone menu opens.
        let timer = Timer(timeInterval: 0.05, repeats: false) { _ in
            MainActor.assumeIsolated {
                for event in events { NSApplication.shared.postEvent(event, atStart: true) }
            }
        }
        RunLoop.current.add(timer, forMode: .eventTracking)
        defer { timer.invalidate() }
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        surface.editorView.mouseDown(with: down)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
        let menu = try XCTUnwrap(tracker.menu, "clicking the language furniture must start the actual native menu")
        XCTAssertEqual(menu.items.first?.title, "None")
        XCTAssertTrue(menu.items[1].isSeparatorItem)
        XCTAssertTrue(menu.items.filter { !$0.isSeparatorItem }.allSatisfy { $0.isEnabled == enabled },
            "language authoring choices must follow the document's editing policy")
        if keys.contains(where: { $0.0 == 36 }) {
            XCTAssertEqual(tracker.highlighted.last, "None", "native menu highlights: \(tracker.highlighted)")
        }
    }

    func testNativeCodeLanguageMenuEscapeAndNoneSelectionPreserveCaretAndUndo() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-menu-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        let source = "before\n\n```rust\nfn main() {}\n```\n\nafter"
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: 350)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 100, width: 850, height: 350),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        defer { window.close() }
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.formattingToolbar.formattedView.performClick(nil)
        let session = try XCTUnwrap(surface.session)
        let caret = surface.viewPresentation.cursor_utf8_offset
        let mode = surface.viewPresentation.mode
        try trackLanguagePicker(surface, keys: [(53, "\u{1b}")])
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        try trackLanguagePicker(surface, keys: [(115, "\u{f729}"), (36, "\r")])
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.replacingOccurrences(of: "```rust", with: "```").utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let start = Int(label.label_byte_start), end = start + Int(label.label_byte_length)
        XCTAssertEqual(String(decoding: snapshot.decorationLabels[start..<end], as: UTF8.self), "None ▾")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("u").value)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        try backend.setReadOnly(true)
        try trackLanguagePicker(surface, keys: [(53, "\u{1b}")], enabled: false)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testWysiwygLanguageFurnitureAndSourceLocalUndoableSelection() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        let source = Data("before\n\n```rust\nfn main() {}\n```\n\nafter".utf8)
        try backend.read(source: source, typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 700, height: 400)
        surface.formattingToolbar.formattedView.performClick(nil)
        let snapshot = try session.layoutExport()
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let start = Int(label.label_byte_start), end = start + Int(label.label_byte_length)
        XCTAssertEqual(String(decoding: snapshot.decorationLabels[start..<end], as: UTF8.self), "Rust ▾")
        let row = try XCTUnwrap(snapshot.rows.first { $0.row_index == label.row_index })
        XCTAssertLessThanOrEqual(label.typographic_bounds.y + label.typographic_bounds.height, row.y + 0.01)
        let state = try backend.documentState()
        for _ in 0..<200 {
            backend.pollSyntax()
            if surface.layoutPaint?.runs.contains(where: { $0.text_start == row.text_start && $0.text_end >= row.text_start + 2 && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0 }) == true { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(surface.layoutPaint?.runs.contains(where: { $0.text_start == row.text_start && $0.text_end >= row.text_start + 2 && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0 }) == true)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        _ = try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision, offset: row.text_start, language: "python")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(String(decoding: source, as: UTF8.self).replacingOccurrences(of: "```rust", with: "```python").utf8))
        XCTAssertThrowsError(try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision, offset: row.text_start, language: ""))
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("u").value)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    }

    func testCodeLanguageMenuKeepsScrolledViewportAwayFromCaretDuringSyntaxRefresh() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-scroll-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (EVStyleTestFixtures.character("syntax:Keyword"), .characterSize, .float(40)),
        ])
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (EVStyleTestFixtures.block("Code Block"), .characterSize, .float(14)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = String(repeating: "before\n\n", count: 100)
            + "```rust\n" + String(repeating: "fn main() {}\n", count: 12) + "```\n\n"
            + String(repeating: "after\n\n", count: 100)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
        let codeStyles = try EVCodeStyleSession(configuration: configuration)
        XCTAssertEqual(try codeStyles.snapshot().definition(for: EVStyleTestFixtures.character("syntax:Keyword"))?.properties[.characterSize]?.declared, .float(40))
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: 350)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 100, width: 850, height: 350),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        defer { window.close() }
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.formattingToolbar.formattedView.performClick(nil)
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.goToLine(101) }
        let codeLayout = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(codeLayout.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let row = try XCTUnwrap(codeLayout.rows.first { $0.row_index == label.row_index })
        let top = CGFloat(label.typographic_bounds.y - 60)
        surface.performInput { _ = try session.goToLine(1) }
        surface.requestVerticalViewport(top: top)
        for _ in 0..<30 {
            backend.pollSyntax()
            try await Task.sleep(for: .milliseconds(5))
        }
        let before = surface.viewportState
        let caret = surface.viewPresentation.cursor_utf8_offset
        let mode = surface.viewPresentation.mode
        XCTAssertEqual(caret, 0)
        XCTAssertGreaterThan(before.top, 100)
        func assertScrollPreserved() {
            XCTAssertEqual(surface.viewportState.top, before.top, accuracy: 0.01)
            XCTAssertEqual(surface.viewportState.left, before.left, accuracy: 0.01)
            XCTAssertEqual(surface.editorView.viewportOrigin.y, CGFloat(before.top), accuracy: 0.01)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
            XCTAssertEqual(surface.viewPresentation.mode, mode)
        }
        try trackLanguagePicker(surface, keys: [(115, "\u{f729}"), (36, "\r")])
        assertScrollPreserved()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data(source.replacingOccurrences(of: "```rust", with: "```").utf8))
        // The reverse change also has a visible caret. Its delayed keyword
        // capture grows from the declared 14-point base to 40 points; applying
        // those metrics must not pin the caret baseline by changing scroll.
        surface.performInput { _ = try session.goToLine(101) }
        let visibleBefore = surface.viewportState
        let visibleCaret = surface.viewPresentation.cursor_utf8_offset
        XCTAssertEqual(visibleCaret, row.text_start)
        let visibleLayout = try XCTUnwrap(surface.layoutSnapshot)
        let visibleRow = try XCTUnwrap(visibleLayout.rows.first { $0.text_start == visibleCaret })
        XCTAssertGreaterThanOrEqual(visibleRow.baseline, visibleBefore.top)
        XCTAssertLessThanOrEqual(visibleRow.baseline, visibleBefore.top + visibleLayout.info.viewport_height)
        XCTAssertNotNil(visibleLayout.clusters.first { $0.text_start == visibleCaret },
            "the font publication assertion must start with an actually visible code token")
        func assertVisibleScrollPreserved() {
            XCTAssertEqual(surface.viewportState.top, visibleBefore.top, accuracy: 0.01)
            XCTAssertEqual(surface.viewportState.left, visibleBefore.left, accuracy: 0.01)
            XCTAssertEqual(surface.editorView.viewportOrigin.y, CGFloat(visibleBefore.top), accuracy: 0.01)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, visibleCaret)
            XCTAssertEqual(surface.viewPresentation.mode, mode)
        }
        let state = try backend.documentState()
        surface.performInput {
            _ = try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision,
                offset: row.text_start, language: "rust")
        }
        assertVisibleScrollPreserved()
        var highlighted = false
        var lastSize: CGFloat?
        for _ in 0..<300 {
            backend.pollSyntax()
            assertVisibleScrollPreserved()
            if let cluster = surface.layoutSnapshot?.clusters.first(where: { $0.text_start == row.text_start }),
               let advance = session.provider.renderRegistry.enAdvance(identifier: cluster.render_run.identifier,
                   metricsGeneration: cluster.render_run.metrics_generation),
               advance > 0 {
                lastSize = advance * 2
                if abs(advance * 2 - 40) < 0.01 {
                    highlighted = true
                    break
                }
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(highlighted, "the accepted delayed Rust keyword must use the fixture's 40-point syntax font; last native size: \(String(describing: lastSize))")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }
}
