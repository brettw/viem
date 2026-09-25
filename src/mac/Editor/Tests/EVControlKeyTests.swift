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

}
