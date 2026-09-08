import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVCommandPromptEditingTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var text: String?
        var viemGeneration: UInt64 = 1
        var viemIsWritable: Bool { true }
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { text != nil }
        func viemClearContents() -> Int { text = nil; viemGeneration += 1; return 1 }
        func viemSetString(_ string: String) -> Bool { text = string; viemGeneration += 1; return true }
    }
    private func makeSurface() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("document".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        surface.loadViewIfNeeded(); surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400); surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, try XCTUnwrap(surface.session), window)
    }
    private func key(_ code: UInt16, shift: Bool = false) throws -> NSEvent {
        try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: shift ? [.shift] : [], timestamp: 0, windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code))
    }
    func testShiftSelectionClipboardAndNativeReplacementOnlyEditPrompt() throws {
        let (backend, surface, session, window) = try makeSurface()
        let pasteboard = Pasteboard()
        surface.pasteboard = pasteboard
        surface.performInput { _ = try session.sendText(":café") }
        surface.editorView.keyDown(with: try key(123, shift: true))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 3, length: 1))
        surface.editorView.copyDocumentSelection(nil)
        XCTAssertEqual(pasteboard.viemString(), "é")
        surface.editorView.cutDocumentSelection(nil)
        XCTAssertEqual(surface.commandLine?.text, "caf")
        surface.editorView.selectAll(nil)
        surface.editorView.pasteIntoDocument(nil)
        XCTAssertEqual(surface.commandLine?.text, "é")
        surface.editorView.selectAll(nil)
        surface.editorView.insertText("new", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.commandLine?.text, "new")
        XCTAssertEqual(try backend.formattedText(), "document")
        XCTAssertFalse(backend.persistenceState.isDirty)
        withExtendedLifetime(window) {}
    }
    func testMousePromptSelectionAndContextMenuKeepDocumentSelectionUntouched() throws {
        let (backend, surface, session, window) = try makeSurface()
        surface.performInput { _ = try session.sendText(":abcdef") }
        let state = try XCTUnwrap(surface.editorView.commandLineRenderState())
        let point = surface.editorView.convert(NSPoint(x: state.textOrigin.x, y: state.bandRect.midY), to: nil)
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        surface.editorView.mouseDown(with: down)
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, 0)
        surface.editorView.keyDown(with: try key(119, shift: true))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 6))
        let menu = try XCTUnwrap(surface.editorView.menu(for: down))
        XCTAssertEqual(menu.items.map(\.title), ["Cut", "Copy", "Paste", "Paste and Match Style", "Select All"])
        XCTAssertTrue(surface.editorView.validateMenuItem(menu.items[0]))
        XCTAssertEqual(try backend.formattedText(), "document")
        withExtendedLifetime(window) {}
    }
    func testCommandOutputIsSelectablePersistentClosableAndColonReplacesIt() throws {
        let (_, surface, session, window) = try makeSurface()
        surface.publishHostMessage("first\nsecond")
        XCTAssertFalse(surface.editorView.commandOutputBar.isHidden)
        XCTAssertFalse(surface.editorView.commandOutputBar.textView.isEditable)
        XCTAssertTrue(surface.editorView.commandOutputBar.textView.isSelectable)
        surface.performInput { _ = try session.sendText("l") }
        XCTAssertEqual(surface.commandOutput, "first\nsecond")
        surface.performInput { _ = try session.sendText(":") }
        XCTAssertNil(surface.commandOutput)
        XCTAssertTrue(surface.editorView.commandOutputBar.isHidden)
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.publishHostMessage("new")
        surface.dismissCommandOutput()
        XCTAssertNil(surface.commandOutput)
        withExtendedLifetime(window) {}
    }
    func testNativeOptionWarningsAndExErrorsReachPersistentOutput() throws {
        let (_, surface, session, window) = try makeSurface()
        surface.performInput { _ = try session.sendText(":s bar.txt"); _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
        XCTAssertTrue(surface.commandOutput?.contains(":saveas <file>") == true)
        surface.performInput { _ = try session.sendText("i🙂"); _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        let state = try surface.backend.documentState()
        surface.performInput { _ = try session.setEncoding(UInt32(VIEM_ENCODING_LATIN1), expected: state) }
        XCTAssertNotNil(surface.commandOutput)
        XCTAssertTrue(surface.commandOutput?.lowercased().contains("replac") == true)
        XCTAssertFalse(surface.editorView.commandOutputBar.isHidden)
        XCTAssertEqual(surface.statusBarState.message, "")
        withExtendedLifetime(window) {}
    }

    func testStalePromptSnapshotCannotOverwriteSelection() throws {
        let (_, surface, session, window) = try makeSurface()
        surface.performInput { _ = try session.sendText(":abc") }
        let stale = try session.commandLineExport()
        _ = try session.editCommandLine(stale, anchor: 0, active: 2)
        XCTAssertThrowsError(try session.editCommandLine(stale, selecting: 0..<3, replacement: "bad"))
        XCTAssertEqual(try session.commandLineExport().text, "abc")
        withExtendedLifetime(window) {}
    }
}
