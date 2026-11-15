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
        try backend.readUntitled(source: Data(text.utf8), typeName: EVDocument.plainTextType)
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

    func testShiftArrowSelectionIsIndependentOfInsertSelectionPreference() throws {
        for allowInsert in [false, true] {
            for startSelection in [false, true] {
                for insertMode in [false, true] {
                    let config = configuration()
                    try config.setAutoselect(allowInsert)
                    try config.setShiftselect(startSelection)
                    let (_, surface, session, window) = try fixture("one two", configuration: config)
                    defer { window.close() }
                    if insertMode { surface.performInput { _ = try session.sendText("i") } }
                    surface.editorView.keyDown(with: try arrow())
                    if startSelection {
                        XCTAssertEqual(surface.viewPresentation.mode, UInt32(insertMode ? VIEM_MODE_SELECTION_CHARACTER : VIEM_MODE_VISUAL_CHARACTER))
                        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: insertMode ? 1 : 2))
                    } else {
                        XCTAssertEqual(surface.viewPresentation.mode, UInt32(insertMode ? VIEM_MODE_INSERT : VIEM_MODE_NORMAL))
                        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
                        XCTAssertTrue(surface.selectedUTF8Ranges().isEmpty)
                    }
                }
            }
        }
    }

    func testStartupShiftArrowMappingsOverrideBothSettingsAndNativeSelection() throws {
        let config = configuration()
        try FileManager.default.createDirectory(at: config.directory, withIntermediateDirectories: true)
        try Data("nnoremap <S-Right> l\ninoremap <S-Right> <Left>\n".utf8)
            .write(to: config.directory.appendingPathComponent("startup.viem"))
        defer { try? FileManager.default.removeItem(at: config.directory) }
        let (_, surface, session, window) = try fixture("one two", configuration: config)
        defer { window.close() }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
        surface.editorView.keyDown(with: try arrow(125))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
    }

    func testChangingSelectionDefaultsPreservesOtherLiveOverrides() throws {
        let config = configuration()
        try FileManager.default.createDirectory(at: config.directory, withIntermediateDirectories: true)
        try Data("set noautoselect noshiftselect\n".utf8)
            .write(to: config.directory.appendingPathComponent("startup.viem"))
        defer { try? FileManager.default.removeItem(at: config.directory) }
        let (first, surface, session, window) = try fixture(configuration: config)
        let (second, _, _, secondWindow) = try fixture(configuration: config)
        defer { window.close(); secondWindow.close() }
        try config.setSmartHome(false)
        for backend in [first, second] {
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: backend), "0")
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SHIFTSELECT), from: backend), "0")
        }
        try config.setShiftselect(false)
        try config.setShiftselect(true)
        for backend in [first, second] {
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: backend), "0")
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SHIFTSELECT), from: backend), "1")
        }
        surface.performInput { _ = try session.executeCommandLine("set noshiftselect") }
        try config.setAutoselect(false)
        try config.setAutoselect(true)
        for backend in [first, second] {
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: backend), "1")
            XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SHIFTSELECT), from: backend), "0")
        }
    }

    func testShiftArrowsReplaceWholeGraphemeAndUndoOnce() throws {
        let source = "👩‍💻e\u{301} end"
        let (backend, surface, session, window) = try fixture(source)
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
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
        let (_, surface, session, window) = try fixture()
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.doCommand(by: #selector(NSResponder.moveRightAndModifySelection(_:)))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 1))
        surface.editorView.keyDown(with: try arrow(124, [.command, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 7))
        surface.editorView.keyDown(with: try arrow(125, [.command, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 12))
    }

    func testUnshiftedArrowsCollapseSelectionLikeAppKit() throws {
        for key: UInt16 in [123, 124] {
            let (_, surface, session, window) = try fixture()
            defer { window.close() }
            surface.performInput { _ = try session.sendText("i") }
            surface.editorView.keyDown(with: try arrow())
            surface.editorView.keyDown(with: try arrow())
            surface.editorView.keyDown(with: try arrow(key, []))
            XCTAssertTrue(surface.selectedUTF8Ranges().isEmpty)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, key == 123 ? 0 : 2)
        }
    }

    func testOptionArrowsNavigateParagraphsAcrossWrappedRowsAndExtendSelection() throws {
        let source = String(repeating: "wrapped words ", count: 30) + "\n\nsecond paragraph\nlast"
        let (backend, surface, session, window) = try fixture(source)
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 4, length: 0))
        let firstEnd = (source as NSString).range(of: "\n").location
        surface.editorView.keyDown(with: try arrow(125, [.option]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: firstEnd, length: 0))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        surface.editorView.keyDown(with: try arrow(126, [.option]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 0))
        surface.editorView.keyDown(with: try arrow(125, [.option, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: firstEnd))
        surface.editorView.keyDown(with: try arrow(125, [.option, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: firstEnd + 1))
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfParagraphAndModifySelection(_:)))
        let secondEnd = (source as NSString).range(of: "\nlast").location
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: secondEnd))
        surface.editorView.keyDown(with: try arrow(126, [.option, .shift]))
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: firstEnd + 2))
        XCTAssertEqual(try backend.formattedText(), source)
    }

    func testAccessibilityRangesRemainExactWhenReselectingFromInsertSelection() throws {
        let (_, surface, session, window) = try fixture("one 👩‍💻 end")
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
        for range in [NSRange(location: 0, length: 3), NSRange(location: 4, length: 5), NSRange(location: 10, length: 3)] {
            surface.editorView.setAccessibilitySelectedTextRange(range)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
            XCTAssertEqual(surface.editorView.accessibilitySelectedTextRange(), range)
        }
    }

    func testAccessibilityRangesKeepExactEndsInSelectAndVisualModes() throws {
        for selectmode in ["mouse,key", ""] {
            let (_, surface, session, window) = try fixture("one 👩‍💻 end")
            defer { window.close() }
            surface.performInput { _ = try session.executeCommandLine("set noautoselect selectmode=\(selectmode)") }
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
        surface.performInput { _ = try session.sendText("i") }
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
        let (backend, surface, session, window) = try fixture()
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
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
        surface.performInput { _ = try session.executeCommandLine("set noautoselect noshiftselect selectmode= keymodel=startsel") }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        surface.editorView.keyDown(with: try arrow(124, []))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.executeCommandLine("set selectmode=cmd,key keymodel=")
        }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        surface.performInput { _ = try session.sendTestKeys("0v") }
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
        first.performInput { _ = try firstSession.executeCommandLine("set noautoselect noshiftselect km= slm=cmd") }
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_KEYMODEL), from: secondBackend), "")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SELECTMODE), from: secondBackend), "cmd")
        XCTAssertEqual(second.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: secondBackend), "0")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SHIFTSELECT), from: secondBackend), "0")
        let (thirdBackend, _, _, thirdWindow) = try fixture(configuration: config)
        defer { thirdWindow.close() }
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_KEYMODEL), from: thirdBackend), "")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SELECTMODE), from: thirdBackend), "cmd")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_AUTOSELECT), from: thirdBackend), "0")
        XCTAssertEqual(try EVSelectionPreferences.read(UInt32(VIEM_EX_OPTION_SHIFTSELECT), from: thirdBackend), "0")
    }

    func testStatusDistinguishesNativeSelectionFromVimVisualAndSelect() throws {
        let (_, surface, session, window) = try fixture()
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
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
            _ = try session.sendTestKeys("gh")
        }
        XCTAssertEqual(surface.statusBarState.mode, "SELECT")
    }

    func testNoAutoselectUsesVimInclusiveSelectionAndDeleteReturnsNormal() throws {
        let (backend, surface, session, window) = try fixture("abcd")
        defer { window.close() }
        surface.performInput {
            _ = try session.executeCommandLine("set noautoselect noshiftselect keymodel=startsel,stopsel selectmode=key")
        }
        surface.editorView.keyDown(with: try arrow())
        XCTAssertEqual(surface.statusBarState.mode, "SELECT")
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 2))
        surface.editorView.doCommand(by: #selector(NSResponder.deleteForward(_:)))
        XCTAssertEqual(try backend.formattedText(), "cd")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }

    func testMenuSelectionFromNormalAndReplaceUsesVisualMode() throws {
        for command in ["", "R"] {
            let (_, surface, session, window) = try fixture("one two")
            defer { window.close() }
            if !command.isEmpty { surface.performInput { _ = try session.sendText(command) } }
            surface.perform(menuCommand: .selectWord, sender: nil)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "one")
        }
    }

    func testNativeMenuSelectionPreservesInsertionBoundaryAndResumeMode() throws {
        let (_, surface, session, window) = try fixture("one two")
        defer { window.close() }
        surface.performInput { _ = try session.sendTestKeys("wi") }
        surface.perform(menuCommand: .selectWord, sender: nil)
        XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "two")
        XCTAssertEqual(surface.statusBarState.mode, "SELECTION")
        surface.editorView.keyDown(with: try arrow(124, []))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)
    }
}
