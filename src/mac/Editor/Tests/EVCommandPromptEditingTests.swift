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
        let container = NSViewController()
        container.view = NSView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        window.contentViewController = container
        container.addChild(surface)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: EVStatusBarView.preferredHeight, width: 600, height: 400 - EVStatusBarView.preferredHeight)
        container.view.addSubview(surface.view)
        surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        // A real pane pairs the editor with a status line, which now renders
        // and hit-tests the command line.
        let statusBar = EVStatusBarView()
        statusBar.frame = NSRect(x: 0, y: 0, width: 600, height: EVStatusBarView.preferredHeight)
        window.contentView?.addSubview(statusBar)
        statusBar.apply(surface.statusBarState)
        surface.statusBarStateDidChange = { [weak statusBar, weak surface, weak window] state in
            let outputFocused = statusBar?.isCommandOutputFocused == true
            statusBar?.apply(state)
            if outputFocused, state.commandOutput == nil { window?.makeFirstResponder(surface?.view) }
        }
        statusBar.commandOutputDidDismiss = { [weak surface] in surface?.dismissCommandOutput() }
        statusBar.commandOutputDidReceiveKey = { [weak surface] event in surface?.handleStatusMessageKey(event) }
        statusBar.commandLineDidSelect = { [weak surface] offset, extending in
            surface?.selectCommandLine(atUTF8Offset: offset, extending: extending)
        }
        statusBar.layoutSubtreeIfNeeded()
        return (backend, surface, try XCTUnwrap(surface.session), window)
    }

    private func statusBar(in window: NSWindow) throws -> EVStatusBarView {
        try XCTUnwrap(window.contentView?.subviews.compactMap { $0 as? EVStatusBarView }.first)
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
        _ = try XCTUnwrap(surface.editorView.commandLineRenderState())
        // The command line lives in the status line, so clicking it happens
        // there rather than over the document.
        let statusBar = try statusBar(in: window)
        statusBar.layoutSubtreeIfNeeded()
        let point = statusBar.convert(
            NSPoint(x: EVStatusBarView.contentInset, y: statusBar.bounds.midY), to: nil)
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        statusBar.mouseDown(with: down)
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, 0)
        surface.editorView.keyDown(with: try key(119, shift: true))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 6))
        let menu = try XCTUnwrap(surface.editorView.menu(for: down))
        XCTAssertEqual(menu.items.map(\.title), ["Cut", "Copy", "Paste", "Paste and Match Style", "Select All"])
        XCTAssertTrue(surface.editorView.validateMenuItem(menu.items[0]))
        XCTAssertEqual(try backend.formattedText(), "document")
        withExtendedLifetime(window) {}
    }
    private func outputTextView(in bar: EVStatusBarView) throws -> NSTextView {
        try XCTUnwrap(bar.subviews.compactMap { ($0 as? NSScrollView)?.documentView as? NSTextView }.first)
    }

    func testCommandOutputUsesStatusLineWithoutMovingDocumentAndDismissesOnInput() throws {
        let (backend, surface, session, window) = try makeSurface()
        let bar = try statusBar(in: window)
        let frame = surface.view.frame
        let viewport = surface.editorView.layoutViewportSize
        let text = try outputTextView(in: bar)
        surface.publishHostMessage("first\nsecond")
        XCTAssertEqual(surface.statusBarState.commandOutput, "first\nsecond")
        XCTAssertFalse(text.isHiddenOrHasHiddenAncestor)
        XCTAssertFalse(text.isEditable)
        XCTAssertTrue(text.isSelectable)
        XCTAssertEqual(surface.view.frame, frame)
        XCTAssertEqual(surface.editorView.layoutViewportSize, viewport)
        surface.performInput { _ = try session.sendText("l") }
        XCTAssertNil(surface.commandOutput)
        XCTAssertTrue(text.isHiddenOrHasHiddenAncestor)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        surface.publishHostMessage("new")
        surface.performInput { _ = try session.sendText(":") }
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(surface.commandLine?.prompt, ":")
        XCTAssertEqual(try backend.formattedText(), "document")
        XCTAssertEqual(surface.view.frame, frame)
        withExtendedLifetime(window) {}
    }

    func testOutputSelectionCopiesExactMultilineUnicodeAndSurvivesPresentationRefresh() throws {
        let (backend, surface, _, window) = try makeSurface()
        let bar = try statusBar(in: window)
        let text = try outputTextView(in: bar)
        let message = "café 🙂\nsecond line"
        surface.publishHostMessage(message)
        window.makeFirstResponder(text)
        surface.perform(menuCommand: .selectAll, sender: nil)
        XCTAssertTrue(surface.presentation(for: .copy).isEnabled)
        XCTAssertFalse(surface.presentation(for: .cut).isEnabled)
        let selected = text.selectedRange()
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        // NSTextView advertises its supported pasteboard spelling (currently
        // NSStringPboardType); requesting .string directly bypasses that native
        // negotiation and fails even for an ordinary stock NSTextView.
        XCTAssertTrue(text.writeSelection(to: pasteboard, types: text.writablePasteboardTypes))
        XCTAssertEqual(pasteboard.string(forType: .string), message)
        surface.refreshStatusBarActivity()
        XCTAssertEqual(text.selectedRange(), selected)
        XCTAssertEqual(surface.commandOutput, message)
        surface.perform(menuCommand: .cut, sender: nil)
        XCTAssertEqual(text.string, message)
        XCTAssertEqual(try backend.formattedText(), "document")
        let event = try XCTUnwrap(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "l",
            charactersIgnoringModifiers: "l", isARepeat: false, keyCode: 37))
        text.keyDown(with: event)
        XCTAssertNil(surface.commandOutput)
        XCTAssertTrue(window.firstResponder === surface.editorView)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        XCTAssertEqual(try backend.formattedText(), "document")
    }

    func testOutputTimeoutUsesNewMessageDeadlineAndPreservesOtherFocus() throws {
        let (_, surface, _, window) = try makeSurface()
        let bar = try statusBar(in: window)
        let text = try outputTextView(in: bar)
        var now: TimeInterval = 100
        surface.commandOutputClock = { now }
        surface.publishHostMessage("old")
        XCTAssertEqual(surface.commandOutputDeadline, 130)
        surface.expireCommandOutput(at: 129.9)
        XCTAssertEqual(surface.commandOutput, "old")
        now = 120
        surface.publishHostMessage("new")
        XCTAssertEqual(surface.commandOutputDeadline, 150)
        surface.expireCommandOutput(at: 130)
        XCTAssertEqual(surface.commandOutput, "new")
        window.makeFirstResponder(text)
        surface.expireCommandOutput(at: 150)
        XCTAssertNil(surface.commandOutput)
        XCTAssertNil(surface.commandOutputDeadline)
        XCTAssertTrue(window.firstResponder === surface.editorView)
        surface.publishHostMessage("last")
        let other = NSTextField(frame: NSRect(x: 0, y: 0, width: 80, height: 24))
        window.contentView?.addSubview(other)
        window.makeFirstResponder(other)
        let previousResponder = window.firstResponder
        surface.expireCommandOutput(at: 200)
        XCTAssertTrue(window.firstResponder === previousResponder)
        surface.publishHostMessage("close me")
        let close = try XCTUnwrap(bar.subviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "Close command output" })
        close.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertNil(surface.commandOutputDeadline)
    }
    func testNativeOptionWarningsAndExErrorsReachStatusOutput() throws {
        let (_, surface, session, window) = try makeSurface()
        surface.performInput { _ = try session.sendText(":s bar.txt"); _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
        XCTAssertTrue(surface.commandOutput?.contains(":saveas <file>") == true)
        surface.performInput { _ = try session.sendText("i🙂"); _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        let state = try surface.backend.documentState()
        surface.performInput { _ = try session.setEncoding(UInt32(VIEM_ENCODING_LATIN1), expected: state) }
        XCTAssertNotNil(surface.commandOutput)
        XCTAssertTrue(surface.commandOutput?.lowercased().contains("replac") == true)
        XCTAssertEqual(surface.statusBarState.commandOutput, surface.commandOutput)
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
