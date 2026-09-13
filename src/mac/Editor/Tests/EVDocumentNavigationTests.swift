import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVDocumentNavigationTests: XCTestCase {
    private func makeSurface(_ text: String, width: CGFloat = 600, typeName: String = "public.plain-text") throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(text.utf8), typeName: typeName)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: width, height: 400)
        surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, try XCTUnwrap(surface.session), window)
    }

    private func key(_ code: UInt16, control: Bool) throws -> NSEvent {
        let character = code == 115 ? "\u{F729}" : "\u{F72B}"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: control ? [.control, .function] : [.function], timestamp: 0,
            windowNumber: 0, context: nil, characters: character,
            charactersIgnoringModifiers: character, isARepeat: false, keyCode: code))
    }

    func testControlHomeAndEndNavigateDocumentWhileLineSelectorsStayOnLine() throws {
        let source = "first line\nmiddle line\nlast line"
        let (backend, surface, session, window) = try makeSurface(source)
        surface.performInput { _ = try session.sendText("jll") }
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfLine(_:)))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64("first line\nmiddle lin".utf8.count))
        surface.editorView.keyDown(with: try key(119, control: true))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(source.utf8.count - 1))
        surface.editorView.doCommand(by: #selector(NSResponder.moveToBeginningOfLine(_:)))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64("first line\nmiddle line\n".utf8.count))
        surface.editorView.keyDown(with: try key(115, control: true))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(surface.statusBarState.message, "")
        withExtendedLifetime(window) {}
    }

    func testDocumentSelectorsPreserveInsertModeAndNavigatePromptWhenItIsActive() throws {
        let source = "first\nlast"
        let (backend, surface, session, window) = try makeSurface(source)
        surface.performInput { _ = try session.sendText("li") }
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfDocument(_:)))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(source.utf8.count))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        surface.editorView.doCommand(by: #selector(NSResponder.moveToBeginningOfDocument(_:)))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendText(":edit name")
        }
        surface.editorView.keyDown(with: try key(115, control: true))
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, 0)
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfDocument(_:)))
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, UInt64("edit name".utf8.count))
        XCTAssertEqual(try backend.formattedText(), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        withExtendedLifetime(window) {}
    }

    func testHomeAndEndUseAppKitKeyBindingsAndControlKeysStayExplicit() throws {
        let source = "first\nmiddle\nlast"
        let (backend, surface, session, window) = try makeSurface(source)
        let view = KeyBindingEditorView(surface: surface)
        for insertMode in [false, true] {
            for binding in KeyBindingEditorView.Binding.allCases {
                surface.performInput {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_DOCUMENT_START))
                    _ = try session.sendText("jll")
                    if insertMode { _ = try session.sendText("i") }
                }
                view.binding = binding
                view.keyDown(with: try key(119, control: false))
                let endOffset = binding == .document ? 17 : 12
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(endOffset - (insertMode ? 0 : 1)))
                view.keyDown(with: try key(115, control: false))
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, binding == .document ? 0 : 6)
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(insertMode ? VIEM_MODE_INSERT : VIEM_MODE_NORMAL))
            }
            let interpretedCount = view.interpretedKeys.count
            view.keyDown(with: try key(119, control: true))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, insertMode ? 17 : 16)
            XCTAssertEqual(view.interpretedKeys.count, interpretedCount)
        }
        XCTAssertEqual(view.interpretedKeys, Array(repeating: [UInt16(119), 115], count: 6).flatMap { $0 })
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
        XCTAssertEqual(surface.statusBarState.message, "")
        withExtendedLifetime(window) {}
    }

    func testMarkdownInsertLineAndParagraphSelectorsFollowWrappedRowsAndPhysicalMode() throws {
        let source = "one two three four five six seven eight nine ten eleven twelve\nTail"
        let selectors = [
            (#selector(NSResponder.moveToBeginningOfLine(_:)), #selector(NSResponder.moveToEndOfLine(_:))),
            (#selector(NSResponder.moveToBeginningOfParagraph(_:)), #selector(NSResponder.moveToEndOfParagraph(_:))),
        ]
        for (beginning, end) in selectors {
            let (backend, surface, session, window) = try makeSurface(source, width: 240, typeName: EVDocument.markdownType)
            let row = try XCTUnwrap(surface.layoutSnapshot?.rows.dropFirst().first)
            XCTAssertGreaterThan(row.text_start, 0)
            XCTAssertLessThan(row.text_end, UInt64(source.utf8.count - 5))
            surface.performInput {
                let layout = try XCTUnwrap(surface.layoutSnapshot)
                let geometry = try session.caretGeometry(offset: row.text_start + 1,
                    affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: layout.info)
                _ = try session.placeCursor(geometry.point, extendSelection: false)
                _ = try session.sendText("i")
            }
            surface.editorView.doCommand(by: beginning)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, row.text_start)
            surface.editorView.doCommand(by: end)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, row.text_end)
            XCTAssertEqual(surface.viewPresentation.cursor_affinity, UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM))
            surface.performInput { try session.setLineMode(.physicalSource) }
            surface.editorView.doCommand(by: beginning)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
            surface.editorView.doCommand(by: end)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(source.utf8.count - 5))
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertFalse(surface.canUndo)
            withExtendedLifetime(window) {}
        }
    }

    func testShiftVExportsOnlyTheDisplayedRowInVisualLineMode() throws {
        let source = "one two three four five six seven eight nine ten eleven twelve\nTail"
        let (backend, surface, session, window) = try makeSurface(source, width: 240)
        let row = try XCTUnwrap(surface.layoutSnapshot?.rows.first)
        XCTAssertLessThan(row.text_end, UInt64(source.firstIndex(of: "\n")!.utf16Offset(in: source)))
        surface.performInput { _ = try session.sendText("V") }
        XCTAssertEqual(surface.visualSelection?.segments.first?.text_end, row.text_end)
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            try session.setLineMode(.physicalSource)
            _ = try session.sendText("V")
        }
        XCTAssertEqual(surface.visualSelection?.segments.first?.text_end, UInt64(source.firstIndex(of: "\n")!.utf16Offset(in: source) + 1))
        XCTAssertEqual(try backend.formattedText(), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        withExtendedLifetime(window) {}
    }
}

@MainActor private final class KeyBindingEditorView: EVEditorView {
    enum Binding: CaseIterable {
        case line, paragraph, document
    }

    var interpretedKeys: [UInt16] = []
    var binding = Binding.line

    override func interpretKeyEvents(_ eventArray: [NSEvent]) {
        for event in eventArray {
            interpretedKeys.append(event.keyCode)
            let selector: Selector
            switch binding {
            case .document:
                selector = event.keyCode == 115
                    ? #selector(NSResponder.scrollToBeginningOfDocument(_:))
                    : #selector(NSResponder.scrollToEndOfDocument(_:))
            case .line:
                selector = event.keyCode == 115
                    ? #selector(NSResponder.moveToBeginningOfLine(_:))
                    : #selector(NSResponder.moveToEndOfLine(_:))
            case .paragraph:
                selector = event.keyCode == 115
                    ? #selector(NSResponder.moveToBeginningOfParagraph(_:))
                    : #selector(NSResponder.moveToEndOfParagraph(_:))
            }
            doCommand(by: selector)
        }
    }
}
