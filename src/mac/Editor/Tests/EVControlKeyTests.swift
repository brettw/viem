import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVControlKeyTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-control-keys-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func key(_ code: UInt16, control: String? = nil) throws -> NSEvent {
        let text = control ?? ""
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: control == nil ? [] : [.control], timestamp: 0, windowNumber: 0, context: nil,
            characters: text, charactersIgnoringModifiers: text, isARepeat: false, keyCode: code))
    }

    func testControlEditingAliasesMatchTheirPhysicalKeysInInsertAndReplace() throws {
        for mode in ["i", "R"] {
            for (alias, code) in [("h", UInt16(51)), ("i", 48), ("j", 36), ("m", 36)] {
                let (a, sa) = try fixture("abc")
                let (b, sb) = try fixture("abc")
                for surface in [sa, sb] { surface.editorView.insertText(mode + "x", replacementRange: noReplacement) }
                sa.editorView.keyDown(with: try key(0, control: alias))
                sb.editorView.keyDown(with: try key(code))
                XCTAssertEqual(try a.formattedText(), try b.formattedText(), "\(mode) Ctrl-\(alias)")
                XCTAssertEqual(sa.viewPresentation.cursor_utf8_offset, sb.viewPresentation.cursor_utf8_offset)
                XCTAssertNil(sa.commandOutput)
            }
        }
    }

    func testControlWordArrowsUsePortableWordMovementAndRemainQuotable() throws {
        let (backend, surface) = try fixture("one e\u{301}clair")
        surface.editorView.insertText("i", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(124, control: "\u{f703}"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
        surface.editorView.keyDown(with: try key(124, control: "\u{f703}"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64("one e\u{301}clair".utf8.count))
        surface.editorView.keyDown(with: try key(123, control: "\u{f702}"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
        surface.editorView.keyDown(with: try key(9, control: "v"))
        surface.editorView.keyDown(with: try key(123, control: "\u{f702}"))
        XCTAssertEqual(try backend.formattedText(), "one <C-Left>e\u{301}clair")
        XCTAssertNil(surface.commandOutput)
    }

    func testControlCCancelsCountedInsertAndPromptWithoutRepeatingText() throws {
        let (backend, surface) = try fixture("abc")
        surface.editorView.insertText("3iX", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(8, control: "c"))
        XCTAssertEqual(try backend.formattedText(), "Xabc")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        surface.editorView.insertText(":quit", replacementRange: noReplacement)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        surface.editorView.keyDown(with: try key(8, control: "c"))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.commandLine?.info.identity.kind, UInt32(VIEM_COMMAND_LINE_KIND_NONE))
        XCTAssertEqual(surface.commandLine?.text, "")
        XCTAssertEqual(try backend.formattedText(), "Xabc")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "abc")
    }

    func testNormalControlVerticalAliasesReachFollowingRows() throws {
        let (_, surface) = try fixture("one\ntwo\nthree")
        surface.editorView.keyDown(with: try key(38, control: "j"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
        surface.editorView.keyDown(with: try key(45, control: "n"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 8)
        surface.editorView.keyDown(with: try key(35, control: "p"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
        XCTAssertNil(surface.commandOutput)
    }

    func testControlCCancelsTemporaryNormalModeAndKeepsTheCompletedInsertion() throws {
        let (backend, surface) = try fixture("")
        surface.editorView.insertText("iabc", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(31, control: "o"))
        surface.editorView.keyDown(with: try key(8, control: "c"))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 2)
        XCTAssertEqual(try backend.formattedText(), "abc")
        surface.editorView.insertText(".", replacementRange: noReplacement)
        XCTAssertEqual(try backend.formattedText(), "ababcc")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "abc")
        surface.editorView.insertText("x", replacementRange: noReplacement)
        XCTAssertEqual(try backend.formattedText(), "ab")
        XCTAssertNil(surface.commandOutput)
    }

    func testControlEYCopyAdjacentCharactersWithoutLeavingInsert() throws {
        let (backend, surface) = try fixture("abc\nxyz\n123")
        surface.editorView.insertText("ji", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(16, control: "y"))
        surface.editorView.keyDown(with: try key(14, control: "e"))
        XCTAssertEqual(try backend.formattedText(), "abc\na2xyz\n123")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertNil(surface.commandOutput)
    }

    func testControlGUndoBreakAndJoinedHorizontalMovement() throws {
        let (backend, surface) = try fixture("")
        surface.editorView.insertText("iA", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(5, control: "g"))
        surface.editorView.insertText("uB", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(53))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "A")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "")
        surface.editorView.insertText("iAB", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(5, control: "g"))
        surface.editorView.insertText("U", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(123))
        surface.editorView.insertText("x", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(53))
        XCTAssertEqual(try backend.formattedText(), "AxB")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "")
        XCTAssertNil(surface.commandOutput)
    }

    func testTextAfterControlGUppercaseUConsumesTheMovementJoin() throws {
        let (backend, surface) = try fixture("")
        surface.editorView.insertText("iAB", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(5, control: "g"))
        surface.editorView.insertText("UX", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(123))
        surface.editorView.insertText("y", replacementRange: noReplacement)
        surface.editorView.keyDown(with: try key(53))
        XCTAssertEqual(try backend.formattedText(), "AByX")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "ABX")
        XCTAssertNil(surface.commandOutput)
    }
}
