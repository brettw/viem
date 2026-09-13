import AppKit
import CViemCore
import XCTest
@testable import ViemEditor

@MainActor
final class EVInactiveCaretTests: XCTestCase {
    func testWindowApplicationAndPaneFocusStopAndRestoreEveryCaretMode() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("abc".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 250)
        let window = EVTestFocusWindow(contentRect: surface.view.bounds,
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = surface.view
        surface.viewDidLayout()
        var applicationActive = true
        surface.editorView.applicationIsActive = { applicationActive }
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        let session = try XCTUnwrap(surface.session)
        let other = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 200, height: 100),
            styleMask: [.titled], backing: .buffered, defer: false)
        let otherTextSurface = NSTextView(frame: .zero)
        surface.view.addSubview(otherTextSurface)

        func assertActive(insertMode: Bool, file: StaticString = #filePath, line: UInt = #line) {
            XCTAssertTrue(surface.editorView.isCaretActive, file: file, line: line)
            XCTAssertEqual(surface.editorView.customCaretPresentationForTesting,
                insertMode ? .hidden : .active, file: file, line: line)
            XCTAssertEqual(surface.editorView.isDocumentInsertionIndicatorVisible,
                insertMode, file: file, line: line)
        }
        func assertInactive(file: StaticString = #filePath, line: UInt = #line) {
            XCTAssertFalse(surface.editorView.isCaretActive, file: file, line: line)
            XCTAssertEqual(surface.editorView.customCaretPresentationForTesting,
                .inactiveOutline, file: file, line: line)
            XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible, file: file, line: line)
        }

        for command in ["", "i", "R", "v", "V", "block"] {
            surface.performInput {
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                if command == "block" {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
                } else if !command.isEmpty { _ = try session.sendText(command) }
            }
            assertActive(insertMode: command == "i")

            window.setKeyWindowForTesting(false)
            XCTAssertTrue(window.firstResponder === surface.editorView)
            assertInactive()
            surface.editorView.applyPresentation()
            assertInactive()
            window.setKeyWindowForTesting(true)
            assertActive(insertMode: command == "i")

            // Another window's notifications must not change this view's state.
            other.setKeyWindowForTesting(true)
            other.setKeyWindowForTesting(false)
            assertActive(insertMode: command == "i")

            applicationActive = false
            NotificationCenter.default.post(name: NSApplication.didResignActiveNotification, object: NSApp)
            XCTAssertTrue(window.isKeyWindow)
            XCTAssertTrue(window.firstResponder === surface.editorView)
            assertInactive()
            applicationActive = true
            NotificationCenter.default.post(name: NSApplication.didBecomeActiveNotification, object: NSApp)
            assertActive(insertMode: command == "i")

            XCTAssertTrue(window.makeFirstResponder(otherTextSurface))
            XCTAssertTrue(window.isKeyWindow)
            assertInactive()
            XCTAssertTrue(window.makeFirstResponder(surface.editorView))
            assertActive(insertMode: command == "i")
        }
        withExtendedLifetime((window, other)) {}
    }

    func testInactiveOutlineRetainsModeGeometry() {
        let cell = NSRect(x: 10, y: 20, width: 18, height: 24)
        for mode in [VIEM_MODE_NORMAL, VIEM_MODE_VISUAL_CHARACTER, VIEM_MODE_VISUAL_LINE, VIEM_MODE_VISUAL_BLOCK] {
            XCTAssertEqual(EVEditorView.inactiveCaretRect(cell, mode: UInt32(mode)), cell)
        }
        XCTAssertEqual(EVEditorView.inactiveCaretRect(cell, mode: UInt32(VIEM_MODE_INSERT)), NSRect(x: 10, y: 20, width: 2, height: 24))
        XCTAssertEqual(EVEditorView.inactiveCaretRect(cell, mode: UInt32(VIEM_MODE_REPLACE)), NSRect(x: 10, y: 42, width: 18, height: 2))
    }
}
