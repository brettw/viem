import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVListParagraphTests: XCTestCase {
    private func surface(_ source: String, type: String? = nil, startup: String = "")
        throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-list-paragraph-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data(startup.utf8).write(to: directory.appendingPathComponent("startup.viem"))
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: type ?? EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func key(_ code: UInt16 = 36, modifiers: NSEvent.ModifierFlags = [.option]) throws -> NSEvent {
        let character = code == 76 ? "\u{3}" : "\r"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
            characters: character, charactersIgnoringModifiers: character, isARepeat: false, keyCode: code))
    }

    private func menuItem() -> NSMenuItem {
        let item = NSMenuItem(title: "Insert Paragraph in List Item",
            action: #selector(EVFormattingCommandRouting.performEditorFormattingCommand(_:)), keyEquivalent: "\r")
        item.keyEquivalentModifierMask = [.option]
        item.tag = EVMenuCommand.insertListParagraph.rawValue
        return item
    }

    private func focus(_ surface: EVEditorSurfaceController) -> EVTestFocusWindow {
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        return window
    }

    func testPhysicalOptionReturnAndKeypadEnterSplitWithoutAddingAListMarker() throws {
        for (source, expected) in [
            ("- AlphaBeta\n- Tail", "- Alpha\n\n  Beta\n- Tail"),
            ("3. AlphaBeta\n4. Tail", "3. Alpha\n\n   Beta\n4. Tail"),
        ] {
            for (code, mode) in [(UInt16(36), "i"), (76, "R")] {
                let (backend, surface, session) = try surface(source)
                surface.performInput { _ = try session.sendText(mode) }
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 0))
                let markers = try session.layoutExport().decorationLabels
                let originalMode = surface.viewPresentation.mode
                surface.editorView.keyDown(with: try key(code))
                XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta\nTail")
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(expected.utf8))
                XCTAssertEqual(surface.viewPresentation.mode, originalMode)
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 6)
                let layout = try session.layoutExport()
                XCTAssertEqual(layout.decorationLabels, markers)
                XCTAssertEqual(Set(layout.rows.map(\.paragraph_id)).count, 3)
                XCTAssertNil(surface.commandOutput)
                let (reopened, _, _) = try self.surface(expected)
                XCTAssertEqual(try reopened.formattedText(), "Alpha\nBeta\nTail")
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
                surface.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(expected.utf8))
            }
        }
    }

    func testReturnAndShiftReturnKeepTheirMeaningsInsideTheContinuationParagraph() throws {
        let (backend, surface, session) = try surface("- Alpha")
        surface.performInput { _ = try session.sendText("A") }
        surface.editorView.keyDown(with: try key())
        surface.editorView.insertText("Beta", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta")
        XCTAssertEqual(try session.layoutExport().decorations.count, 1)
        surface.editorView.keyDown(with: try key(modifiers: [.shift]))
        surface.editorView.insertText("Gamma", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta\nGamma")
        XCTAssertEqual(Set(try session.layoutExport().rows.map(\.paragraph_id)).count, 2)
        XCTAssertEqual(try session.layoutExport().decorations.count, 1)
        surface.editorView.keyDown(with: try key(modifiers: []))
        surface.editorView.insertText("Next", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta\nGamma\nNext")
        XCTAssertEqual(try session.layoutExport().decorations.count, 2)
        XCTAssertEqual(Set(try session.layoutExport().rows.map(\.paragraph_id)).count, 3)
        XCTAssertNil(surface.commandOutput)
        surface.editorView.cancelOperation(nil)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- Alpha".utf8))
    }

    func testMenuAndKeyEquivalentEnterInsertFromNormalAndRespectNativeFieldFocus() throws {
        for menuRoute in [false, true] {
            let (backend, surface, _) = try surface("- AlphaBeta\n- Tail")
            let window = focus(surface)
            defer { window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 0))
            let item = menuItem()
            XCTAssertTrue(surface.editorView.validateMenuItem(item))
            if menuRoute {
                XCTAssertTrue(try XCTUnwrap(window.firstResponder).tryToPerform(try XCTUnwrap(item.action), with: item))
            } else {
                XCTAssertTrue(surface.editorView.performKeyEquivalent(with: try key(76)))
            }
            XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta\nTail")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertNil(surface.commandOutput)
            let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
            window.setKeyWindowForTesting(false)
            XCTAssertFalse(surface.editorView.validateMenuItem(item))
            XCTAssertFalse(surface.editorView.performKeyEquivalent(with: try key()))
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
            window.setKeyWindowForTesting(true)
            let field = NSTextField(string: "native field")
            surface.view.addSubview(field)
            XCTAssertTrue(window.makeFirstResponder(field))
            XCTAssertFalse(surface.editorView.validateMenuItem(item))
            XCTAssertFalse(surface.editorView.performKeyEquivalent(with: try key()))
            surface.editorView.performEditorFormattingCommand(item)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
        }
    }

    func testKeyEquivalentRetainsCoreMappingsAndLiteralNext() throws {
        let (backend, surface, session) = try surface("- body", startup: "imap <A-CR> X\n")
        let window = focus(surface)
        defer { window.close() }
        surface.performInput { _ = try session.sendText("i") }
        XCTAssertTrue(surface.editorView.performKeyEquivalent(with: try key()))
        XCTAssertEqual(try backend.formattedText(), "Xbody")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- Xbody".utf8))
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118) }
        XCTAssertTrue(surface.editorView.performKeyEquivalent(with: try key(76)))
        XCTAssertEqual(try backend.formattedText(), "X<A-CR>body")
        XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
        XCTAssertEqual(try session.layoutExport().decorations.count, 1)
        XCTAssertNil(surface.commandOutput)
    }

    func testParagraphMenuDisablesUnsupportedContextsAndSelections() throws {
        for (source, type) in [
            ("body", EVDocument.markdownType),
            ("- body", EVDocument.plainTextType),
            ("- body", EVDocument.codeType),
            ("- ```\n  body\n  ```", EVDocument.markdownType),
            ("- | body |\n  | ---- |", EVDocument.markdownType),
        ] {
            let (backend, surface, _) = try surface(source, type: type)
            let text = try backend.formattedText() as NSString
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "body").location, length: 0))
            XCTAssertFalse(surface.presentation(for: .insertListParagraph).isEnabled, source)
            surface.perform(menuCommand: .insertListParagraph, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), source)
            XCTAssertFalse(surface.canUndo)
        }
        let (backend, surface, _) = try surface("- body")
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 2))
        XCTAssertFalse(surface.presentation(for: .insertListParagraph).isEnabled)
        surface.perform(menuCommand: .insertListParagraph, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- body".utf8))
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 0))
        try backend.setReadOnly(true)
        surface.refreshPresentation()
        XCTAssertFalse(surface.presentation(for: .insertListParagraph).isEnabled)
        XCTAssertFalse(surface.canUndo)
    }

    func testParagraphMenuDoesNotConsumePendingCommandsOrComposition() throws {
        for pending in ["operator", "literal", "prompt", "composition"] {
            let (backend, surface, session) = try surface("- body")
            let window = focus(surface)
            defer { window.close() }
            switch pending {
            case "operator": surface.performInput { _ = try session.sendText("d") }
            case "literal":
                surface.performInput {
                    _ = try session.sendText("i")
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
                }
            case "prompt": surface.performInput { _ = try session.sendText(":") }
            default:
                surface.performInput { _ = try session.sendText("i") }
                surface.editorView.setMarkedText("draft", selectedRange: NSRange(location: 5, length: 0),
                    replacementRange: NSRange(location: NSNotFound, length: 0))
            }
            let before = surface.viewPresentation
            let item = menuItem()
            XCTAssertFalse(surface.editorView.validateMenuItem(item), pending)
            surface.editorView.performEditorFormattingCommand(item)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- body".utf8), pending)
            XCTAssertEqual(surface.viewPresentation.mode, before.mode, pending)
            XCTAssertEqual(surface.viewPresentation.flags, before.flags, pending)
            if pending == "composition" {
                XCTAssertFalse(surface.editorView.performKeyEquivalent(with: try key()))
                XCTAssertTrue(surface.editorView.hasMarkedText())
            }
        }
    }
}
