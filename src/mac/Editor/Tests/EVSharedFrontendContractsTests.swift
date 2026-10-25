import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVSharedFrontendContractsTests: XCTestCase {
    private func fixture() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.readUntitled(source: Data("one two".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }
    private func key(_ character: Character, _ session: EVCoreViewSession) throws {
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: character.unicodeScalars.first!.value)
    }
    func testPendingPrefixRecordingAndNativeActionAvailabilityComeFromCore() throws {
        let (_, surface, session) = try fixture()
        for character in "\"a2d" { try key(character, session) }
        surface.refreshPresentation()
        XCTAssertEqual(surface.statusBarState.message, "\"a2d")
        XCTAssertFalse(session.nativeActionAvailable(UInt32(VIEM_NATIVE_SELECT_WORD)))
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        for character in "qqi" { try key(character, session) }
        surface.refreshPresentation()
        XCTAssertEqual(surface.statusBarState.mode, "Recording @q")
        _ = try session.nativeAction(UInt32(VIEM_NATIVE_SELECT_WORD))
        surface.refreshPresentation()
        XCTAssertEqual(surface.statusBarState.mode, "Recording @q")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        try key("q", session)
        surface.refreshPresentation()
        XCTAssertNotEqual(surface.statusBarState.mode, "Recording @q")
    }
    func testPhysicalKeypadEnterUsesDistinctMapping() throws {
        let (backend, surface, session) = try fixture()
        for character in ":inoremap <kEnter> KEY" { try key(character, session) }
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        try key("i", session)
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.numericPad], timestamp: 0, windowNumber: 0, context: nil,
            characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 76))
        surface.editorView.keyDown(with: event)
        XCTAssertEqual(try backend.formattedText(), "KEYone two")
        XCTAssertNil(keypadCode(36))
        XCTAssertEqual(keypadCode(82), 40)
    }
    func testNonExDiagnosticsReachTheCommandOutput() throws {
        let (_, surface, session) = try fixture()
        try key("U", session)
        surface.refreshPresentation()
        XCTAssertFalse(surface.statusBarState.commandOutput?.isEmpty ?? true)
    }
}
