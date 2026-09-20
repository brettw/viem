import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVDeleteSelectionTests: XCTestCase {
    private func surface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: 600, height: 240)
        view.viewDidLayout()
        return (backend, view)
    }

    private func deleteEvent(keyCode: UInt16, windowNumber: Int = 0) throws -> NSEvent {
        let characters = keyCode == 51 ? "\u{7f}" : "\u{f728}"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
            timestamp: 0, windowNumber: windowNumber, context: nil, characters: characters,
            charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
    }

    private func assertHistory(backend: EVCoreDocumentBackend, view: EVEditorSurfaceController,
                               source: String, type: String, expectedText: String) throws {
        XCTAssertEqual(try backend.formattedText(), expectedText)
        XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertTrue(view.selectedUTF8Ranges().isEmpty)
        XCTAssertNotNil(view.formattedPointInfo(atUTF8Offset: Int(view.viewPresentation.cursor_utf8_offset)))
        XCTAssertNil(view.commandOutput)
        let deleted = try backend.serializedSource(typeName: type)
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        view.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), deleted)
        XCTAssertEqual(try backend.formattedText(), expectedText)
        XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertNil(view.commandOutput)
    }

    func testBothPhysicalDeleteKeysRemoveAccessibilitySelectionMadeFromNormalMode() throws {
        let selected = "👩‍💻e\u{301}"
        let text = "A\(selected)Z"
        for (source, type) in [
            (text, EVDocument.plainTextType),
            ("<p data-keep='yes'>A<b>\(selected)</b>Z</p><!--keep-->", EVDocument.htmlType),
        ] {
            for keyCode: UInt16 in [51, 117] {
                let (backend, view) = try surface(source, type: type)
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
                let range = (text as NSString).range(of: selected)
                view.editorView.setAccessibilitySelectedTextRange(range)
                XCTAssertEqual(view.editorView.accessibilitySelectedTextRange(), range)
                XCTAssertEqual(view.editorView.accessibilitySelectedText(), selected)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                view.editorView.keyDown(with: try deleteEvent(keyCode: keyCode))
                try assertHistory(backend: backend, view: view, source: source, type: type, expectedText: "AZ")
            }
        }
    }

    func testPhysicalDeleteRemovesDoubleClickedWordAndEntersInsertMode() throws {
        let source = "before chosen after"
        let type = EVDocument.plainTextType
        let (backend, view) = try surface(source, type: type)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 240),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = view
        defer { window.close() }
        view.viewDidLayout()
        view.refreshPresentation()
        XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        let session = try XCTUnwrap(view.session)
        let snapshot = try XCTUnwrap(view.layoutSnapshot)
        let geometry = try session.caretGeometry(offset: 9, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
        let local = view.editorView.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(geometry.rect.x), y: CGFloat(geometry.rect.y + geometry.rect.height * 0.5)))
        let point = view.editorView.convert(local, to: nil)
        let click = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: point,
            modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil,
            eventNumber: 1, clickCount: 2, pressure: 1))
        view.editorView.mouseDown(with: click)
        XCTAssertEqual(view.editorView.accessibilitySelectedText(), "chosen")
        view.editorView.keyDown(with: try deleteEvent(keyCode: 51, windowNumber: window.windowNumber))
        try assertHistory(backend: backend, view: view, source: source, type: type, expectedText: "before  after")
    }
}
