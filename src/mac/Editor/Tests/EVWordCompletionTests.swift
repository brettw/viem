import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVWordCompletionTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVTestFocusWindow) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-word-completion-\(UUID())")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 640, height: 400)
        surface.viewDidLayout()
        surface.editorView.applicationIsActive = { true }
        window.setKeyWindowForTesting(true)
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, window)
    }

    private func control(_ character: String) throws -> NSEvent {
        try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.control],
            timestamp: 0, windowNumber: 0, context: nil, characters: character,
            charactersIgnoringModifiers: character, isARepeat: false, keyCode: 0))
    }

    private func finishSearch(_ surface: EVEditorSurfaceController) throws {
        for _ in 0 ..< 1_000 where surface.completion?.isSearching == true {
            surface.pollCompletion()
        }
        XCTAssertFalse(surface.completion?.isSearching == true)
        XCTAssertFalse(surface.isCompletionPolling)
        XCTAssertNil(surface.commandOutput)
    }

    func testControlKeysPublishNativePopupAndEscapeAcceptsThenLeavesInsert() throws {
        let (backend, surface, window) = try fixture("apple apricot\nap")
        defer { window.close() }
        surface.editorView.insertText("GA", replacementRange: noReplacement)
        let before = try backend.formattedText()
        surface.editorView.keyDown(with: try control("p"))
        try finishSearch(surface)
        let completion = try XCTUnwrap(surface.completion)
        let selected = try XCTUnwrap(completion.selectedIndex)
        XCTAssertEqual(Set(completion.items), Set(["apple", "apricot"]))
        XCTAssertEqual(try backend.formattedText(), before, "Preview must leave authoritative text unchanged")
        XCTAssertTrue(surface.editorView.isCompletionPopupVisible)
        XCTAssertEqual(surface.editorView.completionPopupSelectedIndex, selected)
        XCTAssertTrue(window.firstResponder === surface.editorView)
        XCTAssertFalse(surface.editorView.hasMarkedText())
        XCTAssertFalse(try XCTUnwrap(surface.session).hasActiveComposition)
        let presented = try XCTUnwrap(surface.fullPresentedText())
        XCTAssertTrue(presented.hasSuffix(completion.items[selected]))
        XCTAssertEqual(surface.editorView.selectedRange().location, presented.utf16.count)
        XCTAssertEqual(surface.editorView.accessibilityNumberOfCharacters(), presented.utf16.count)

        surface.editorView.cancelOperation(nil)
        XCTAssertEqual(try backend.formattedText(), presented)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
        XCTAssertFalse(surface.completion?.isActive == true)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), before)
    }

    func testTypingAcceptsBackendSelectedCandidateAndRunsOrdinaryInput() throws {
        let (backend, surface, window) = try fixture("alpha alpine\nal")
        defer { window.close() }
        surface.editorView.insertText("GA", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try control("n"))
        try finishSearch(surface)
        surface.editorView.keyDown(with: try control("p"))
        surface.editorView.keyDown(with: try control("n"))
        let preview = try XCTUnwrap(surface.fullPresentedText())
        surface.editorView.insertText("!", replacementRange: noReplacement)
        XCTAssertEqual(try backend.formattedText(), preview + "!")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
        XCTAssertFalse(surface.isCompletionPolling)
        XCTAssertNil(surface.commandOutput)
    }

    func testNativePopupDisplaysSuppliedSelectionWithoutTakingKeyboardFocus() throws {
        let (_, surface, owner) = try fixture("")
        defer { owner.close() }
        let popup = EVCompletionPopup()
        defer { popup.hide() }
        popup.show(items: ["alpha", "beta", "gamma"], selectedIndex: 2, searching: false,
                   truncated: false, at: owner.frame, in: owner, rightToLeft: false)
        XCTAssertEqual(popup.items, ["alpha", "beta", "gamma"])
        XCTAssertEqual(popup.nativeSelectedRow, 2)
        XCTAssertTrue(owner.firstResponder === surface.editorView)
        XCTAssertFalse(popup.window.canBecomeKey)
        XCTAssertFalse(popup.window.canBecomeMain)
        let table = try XCTUnwrap((popup.window.contentView?.subviews.compactMap { $0 as? NSScrollView }.first)?.documentView as? NSTableView)
        table.setAccessibilitySelectedRows([])
        table.rowView(atRow: 0, makeIfNecessary: true)?.setAccessibilitySelected(true)
        XCTAssertEqual(popup.nativeSelectedRow, 2, "Native accessibility must not invent a backend selection")
        popup.show(items: ["alpha", "beta", "gamma"], selectedIndex: nil, searching: false,
                   truncated: false, at: owner.frame, in: owner, rightToLeft: false)
        XCTAssertEqual(popup.nativeSelectedRow, -1)
        popup.hide()
        XCTAssertFalse(popup.isVisible)
        XCTAssertFalse(owner.childWindows?.contains(popup.window) == true)
    }

    func testNativeMarkedTextAcceptsCompletionBeforeStartingComposition() throws {
        let (backend, surface, window) = try fixture("apple apricot\nap")
        defer { window.close() }
        surface.editorView.insertText("GA", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try control("p"))
        try finishSearch(surface)
        let preview = try XCTUnwrap(surface.fullPresentedText())
        XCTAssertFalse(surface.editorView.hasMarkedText())
        surface.editorView.setMarkedText("é", selectedRange: NSRange(location: 1, length: 0), replacementRange: noReplacement)
        XCTAssertTrue(surface.editorView.hasMarkedText())
        XCTAssertTrue(try XCTUnwrap(surface.session).hasActiveComposition)
        XCTAssertFalse(surface.completion?.isActive == true)
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
        XCTAssertEqual(try backend.formattedText(), preview)
        XCTAssertEqual(surface.fullPresentedText(), preview + "é")
        XCTAssertEqual(surface.editorView.markedRange(), NSRange(location: preview.utf16.count, length: 1))
        surface.editorView.insertText("é", replacementRange: noReplacement)
        XCTAssertEqual(try backend.formattedText(), preview + "é")
        XCTAssertFalse(surface.editorView.hasMarkedText())
        XCTAssertNil(surface.commandOutput)
    }

    func testWindowFocusLossAcceptsCompletionAndHidesPopup() throws {
        let (backend, surface, window) = try fixture("apple apricot\nap")
        defer { window.close() }
        surface.editorView.insertText("GA", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try control("p"))
        try finishSearch(surface)
        let preview = try XCTUnwrap(surface.fullPresentedText())
        window.setKeyWindowForTesting(false)
        XCTAssertEqual(try backend.formattedText(), preview)
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
        XCTAssertFalse(surface.isCompletionPolling)
        XCTAssertFalse(surface.completion?.isActive == true)
        window.setKeyWindowForTesting(true)
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
    }

    func testNativeExplicitReplacementAndCommandShortcutAcceptBeforeTheirNormalEffect() throws {
        let (backend, surface, window) = try fixture("apple apricot\nap")
        defer { window.close() }
        surface.editorView.insertText("GA", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try control("p"))
        try finishSearch(surface)
        let preview = try XCTUnwrap(surface.fullPresentedText())
        surface.editorView.setMarkedText("é", selectedRange: NSRange(location: 1, length: 0),
                                        replacementRange: NSRange(location: preview.utf16.count - 1, length: 1))
        XCTAssertEqual(try backend.formattedText(), preview)
        XCTAssertEqual(surface.fullPresentedText(), String(preview.dropLast()) + "é")
        surface.editorView.insertText("é", replacementRange: noReplacement)
        surface.editorView.insertText(" ap", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try control("p"))
        try finishSearch(surface)
        let nextPreview = try XCTUnwrap(surface.fullPresentedText())
        let shortcut = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.command], timestamp: 0, windowNumber: 0, context: nil,
            characters: "s", charactersIgnoringModifiers: "s", isARepeat: false, keyCode: 1))
        XCTAssertFalse(surface.editorView.performKeyEquivalent(with: shortcut), "Save continues through normal AppKit routing")
        XCTAssertEqual(try backend.formattedText(), nextPreview)
        XCTAssertFalse(surface.completion?.isActive == true)
        XCTAssertNil(surface.commandOutput)
    }

    func testLargeDocumentSearchingYieldsAndEscapeStopsNativePolling() throws {
        let source = "al\n" + String(repeating: "unrelated words\n", count: 50_000) + "alphabet"
        let (backend, surface, window) = try fixture(source)
        defer { window.close() }
        surface.editorView.insertText("la", replacementRange: noReplacement)
        backend.resetFormattedAccessCounters()
        surface.editorView.keyDown(with: try control("n"))
        XCTAssertTrue(surface.completion?.isSearching == true)
        XCTAssertTrue(surface.isCompletionPolling)
        surface.pollCompletion()
        XCTAssertTrue(surface.completion?.isSearching == true)
        XCTAssertEqual(backend.formattedAccessCounters.fullRangeReadCalls, 0)
        surface.editorView.cancelOperation(nil)
        XCTAssertFalse(surface.isCompletionPolling)
        XCTAssertFalse(surface.completion?.isActive == true)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        surface.pollCompletion()
        XCTAssertFalse(surface.editorView.isCompletionPopupVisible)
        XCTAssertEqual(backend.formattedAccessCounters.fullRangeReadCalls, 0)
        XCTAssertNil(surface.commandOutput)
    }

    func testBackendWordAnchorKeepsNativePopupStationaryAcrossDifferentCandidateLengths() throws {
        for source in ["alpha alphabetical alpine\nal", "كتاب كتابة كتابات\nكت"] {
            let (_, surface, window) = try fixture(source)
            defer { window.close() }
            let screen = try XCTUnwrap(NSScreen.main?.visibleFrame)
            window.setFrameOrigin(NSPoint(x: screen.midX - 250, y: screen.midY - 100))
            surface.editorView.insertText("GA", replacementRange: noReplacement)
            surface.editorView.keyDown(with: try control("n"))
            try finishSearch(surface)
            let completion = try XCTUnwrap(surface.completion)
            XCTAssertTrue(completion.hasAnchor)
            XCTAssertEqual(completion.rightToLeft, source.hasPrefix("ك"))
            let popup = try XCTUnwrap(window.childWindows?.first { !$0.canBecomeKey })
            let originalFrame = popup.frame
            let anchor = completion.info.anchor_rect
            let scroll = try XCTUnwrap(popup.contentView?.subviews.compactMap { $0 as? NSScrollView }.first)
            let table = try XCTUnwrap(scroll.documentView as? NSTableView)
            let row = try XCTUnwrap(completion.selectedIndex)
            let cell = try XCTUnwrap(table.view(atColumn: 0, row: row, makeIfNecessary: true) as? NSTableCellView)
            XCTAssertEqual(cell.textField?.baseWritingDirection, completion.rightToLeft ? .rightToLeft : .leftToRight)
            for _ in 0 ..< 3 {
                surface.editorView.keyDown(with: try control("n"))
                let next = try XCTUnwrap(surface.completion)
                XCTAssertEqual(next.info.anchor_rect.x, anchor.x, accuracy: 0.01)
                XCTAssertEqual(next.info.anchor_rect.y, anchor.y, accuracy: 0.01)
                XCTAssertEqual(popup.frame.minX, originalFrame.minX, accuracy: 0.5)
                XCTAssertEqual(popup.frame.maxY, originalFrame.maxY, accuracy: 0.5)
            }
        }
    }

    func testNativePopupStaysOnScreenAndMovesAboveCaretNearBottom() {
        let screen = NSRect(x: 100, y: 200, width: 800, height: 600)
        let size = NSSize(width: 250, height: 200)
        let below = EVCompletionPopup.frame(for: size, maximumHeight: 222, anchor: NSRect(x: 300, y: 600, width: 2, height: 20), textInset: 12, rightToLeft: false, visibleFrame: screen)
        XCTAssertLessThan(below.maxY, 600)
        let above = EVCompletionPopup.frame(for: size, maximumHeight: 222, anchor: NSRect(x: 880, y: 205, width: 2, height: 20), textInset: 12, rightToLeft: false, visibleFrame: screen)
        XCTAssertGreaterThan(above.minY, 225)
        XCTAssertEqual(above.maxX, screen.maxX)
        XCTAssertTrue(screen.contains(above))
    }
}
