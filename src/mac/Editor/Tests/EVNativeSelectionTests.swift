import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVNativeSelectionTests: XCTestCase {
    private let noRange = NSRange(location: NSNotFound, length: 0)

    private func configuration() -> EVConfigurationStore {
        EVConfigurationStore(directory: FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-select-\(UUID().uuidString)"), legacyDefaults: nil)
    }

    private func fixture(_ text: String = "one two\nlast", configuration: EVConfigurationStore? = nil) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let backend = EVCoreDocumentBackend(configuration: configuration ?? self.configuration())
        try backend.read(source: Data(text.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 240),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        surface.loadViewIfNeeded()
        surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, try XCTUnwrap(surface.session), window)
    }

    private func arrow(_ code: UInt16 = 124, _ modifiers: NSEvent.ModifierFlags = [.shift]) throws -> NSEvent {
        let characters = code == 123 ? "\u{f702}" : code == 126 ? "\u{f700}" : code == 125 ? "\u{f701}" : "\u{f703}"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers,
            timestamp: 0, windowNumber: 0, context: nil, characters: characters,
            charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code))
    }

    func testShiftArrowsReplaceWholeGraphemeAndUndoOnce() throws {
        let source = "👩‍💻e\u{301} end"
        let (backend, surface, session, window) = try fixture(source)
        defer { window.close() }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 5))
        XCTAssertEqual(surface.viewPresentation.caret_shape, UInt32(VIEM_CARET_SHAPE_BOUNDARY))
        surface.editorView.insertText("猫", replacementRange: noRange)
        XCTAssertEqual(try backend.formattedText(), "猫e\u{301} end")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), source)
    }

    func testSelectorsAndCommandShiftArrowExtendToInsertionBoundaries() throws {
        let (_, surface, _, window) = try fixture()
        defer { window.close() }
        surface.editorView.doCommand(by: #selector(NSResponder.moveRightAndModifySelection(_:)))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 1))
        surface.editorView.keyDown(with: try arrow(124, [.command, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 7))
        surface.editorView.keyDown(with: try arrow(125, [.command, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 12))
    }

    func testUnshiftedArrowsCollapseSelectionLikeAppKit() throws {
        for key: UInt16 in [123, 124] {
            let (_, surface, _, window) = try fixture()
            defer { window.close() }
            surface.editorView.keyDown(with: try arrow())
            surface.editorView.keyDown(with: try arrow())
            surface.editorView.keyDown(with: try arrow(key, []))
            XCTAssertTrue(surface.selectedUTF8Ranges().isEmpty)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, key == 123 ? 0 : 2)
        }
    }

    func testAccessibilityRangesKeepExactEndsInSelectAndVisualModes() throws {
        for selectmode in ["mouse,key", ""] {
            let (_, surface, session, window) = try fixture("one 👩‍💻 end")
            defer { window.close() }
            surface.performInput { _ = try session.sourceLine("set noautoselect selectmode=\(selectmode)", depth: 1) }
            let range = NSRange(location: 4, length: 5)
            surface.editorView.setAccessibilitySelectedTextRange(range)
            XCTAssertEqual(surface.editorView.accessibilitySelectedTextRange(), range)
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "👩‍💻")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(selectmode.isEmpty ? VIEM_MODE_VISUAL_CHARACTER : VIEM_MODE_SELECT_CHARACTER))
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 10, length: 3))
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "end")
        }
    }

    func testSelectCompositionIsOverlayUntilCommitAndCancelKeepsSelection() throws {
        let (backend, surface, session, window) = try fixture("one two")
        defer { window.close() }
        surface.perform(menuCommand: .selectWord, sender: nil)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        surface.editorView.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0), replacementRange: noRange)
        XCTAssertTrue(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "one two")
        surface.editorView.cancelOperation(nil)
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 3))
        surface.editorView.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0), replacementRange: noRange)
        surface.editorView.insertText("仮名", replacementRange: noRange)
        XCTAssertEqual(try backend.formattedText(), "仮名 two")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "one two")
    }

    func testNativeCopyRetainsSelectionThenTypingReplacesIt() throws {
        let (backend, surface, _, window) = try fixture()
        defer { window.close() }
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        surface.pasteboard = EVAppKitPasteboardAccess(pasteboard)
        surface.perform(menuCommand: .selectWord, sender: nil)
        let selected = surface.editorView.selectedRange()
        surface.perform(menuCommand: .copy, sender: nil)
        XCTAssertEqual(pasteboard.string(forType: .string), "one")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        XCTAssertEqual(surface.editorView.selectedRange(), selected)
        surface.editorView.insertText("X", replacementRange: noRange)
        XCTAssertEqual(try backend.formattedText(), "X two\nlast")
    }

    func testNativeBlockCopyRetainsExactSegments() throws {
        let (backend, surface, session, window) = try fixture("abcd\nefgh")
        defer { window.close() }
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        surface.pasteboard = EVAppKitPasteboardAccess(pasteboard)
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 108)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 106)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 103)
        }
        let ranges = surface.selectedUTF8Ranges()
        XCTAssertEqual(ranges.count, 2)
        let selected = surface.selectionText()
        surface.perform(menuCommand: .copy, sender: nil)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECT_BLOCK))
        XCTAssertEqual(surface.selectedUTF8Ranges(), ranges)
        XCTAssertEqual(pasteboard.string(forType: .string), selected)
        XCTAssertEqual(try backend.formattedText(), "abcd\nefgh")
    }

    func testAlternateOptionsAndCommandSelectionsAreIndependent() throws {
        let (_, surface, session, window) = try fixture()
        defer { window.close() }
        surface.performInput { _ = try session.sourceLine("set noautoselect selectmode= keymodel=startsel", depth: 1) }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        surface.editorView.keyDown(with: try arrow(124, []))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sourceLine("set selectmode=cmd,key keymodel=", depth: 1)
        }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        surface.performInput { _ = try session.sendText("0v") }
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECT_CHARACTER))
        surface.perform(menuCommand: .selectWord, sender: nil)
        XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "one")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECT_CHARACTER))
    }

    func testSelectionOptionsAreGlobalAcrossExistingAndNewDocumentsWithoutChangingModes() throws {
        let config = configuration()
        let (_, first, firstSession, firstWindow) = try fixture(configuration: config)
        let (secondBackend, second, secondSession, secondWindow) = try fixture(configuration: config)
        defer { firstWindow.close(); secondWindow.close() }
        second.performInput { _ = try secondSession.sendText("i") }
        first.performInput { _ = try firstSession.sourceLine("set noautoselect km= slm=cmd", depth: 1) }
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_KEYMODEL), from: secondBackend), "")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SELECTMODE), from: secondBackend), "cmd")
        XCTAssertEqual(second.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: secondBackend), "0")
        let (thirdBackend, _, _, thirdWindow) = try fixture(configuration: config)
        defer { thirdWindow.close() }
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_KEYMODEL), from: thirdBackend), "")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SELECTMODE), from: thirdBackend), "cmd")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: thirdBackend), "0")
    }

    func testStatusDistinguishesNativeSelectionFromVimVisualAndSelect() throws {
        let (_, surface, session, window) = try fixture()
        defer { window.close() }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.statusBarState.mode, "SELECTION")
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendText("v")
        }
        surface.editorView.keyDown(with: try arrow())
        surface.editorView.keyDown(with: try arrow(124, []))
        XCTAssertEqual(surface.statusBarState.mode, "VISUAL")
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 1, length: 3))
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendText("gh")
        }
        XCTAssertEqual(surface.statusBarState.mode, "SELECT")
    }

    func testNoAutoselectUsesVimInclusiveSelectionAndDeleteReturnsNormal() throws {
        let (backend, surface, session, window) = try fixture("abcd")
        defer { window.close() }
        surface.performInput {
            _ = try session.sourceLine("set noautoselect keymodel=startsel,stopsel selectmode=key", depth: 1)
        }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.statusBarState.mode, "SELECT")
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 2))
        surface.editorView.doCommand(by: #selector(NSResponder.deleteForward(_:)))
        XCTAssertEqual(try backend.formattedText(), "cd")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }

    func testNativeMenuSelectionPreservesInsertionBoundaryAndResumeMode() throws {
        for (command, mode) in [("i", UInt32(VIEM_MODE_INSERT)), ("R", UInt32(VIEM_MODE_REPLACE))] {
            let (_, surface, session, window) = try fixture("one two")
            defer { window.close() }
            surface.performInput { _ = try session.sendText("w" + command) }
            surface.perform(menuCommand: .selectWord, sender: nil)
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "two")
            XCTAssertEqual(surface.statusBarState.mode, "SELECTION")
            surface.editorView.keyDown(with: try arrow(124, []))
            XCTAssertEqual(surface.viewPresentation.mode, mode)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)
        }
    }
}
