import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVSubstituteConfirmationTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ text: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-confirm-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(text.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 640, height: 260)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func type(_ text: String, _ surface: EVEditorSurfaceController) {
        surface.editorView.insertText(text, replacementRange: noReplacement)
    }

    private func begin(_ command: String, _ surface: EVEditorSurfaceController) throws {
        type(command, surface)
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
        XCTAssertNotNil(surface.substituteConfirmationPrompt, surface.statusBarState.message)
        XCTAssertTrue(surface.statusBarState.message.contains("y/n/a/q/l"))
    }

    func testNativeChoicesDisplayPromptAndCurrentMatchThenPublishOneUndoUnit() throws {
        let (backend, surface) = try fixture("cat cat cat")
        let before = try backend.recoverySnapshot()
        try begin(":s/cat/dog/gc", surface)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        let paint = try XCTUnwrap(surface.layoutPaint)
        XCTAssertTrue(paint.runs.contains { $0.text_start == 0 && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0 })
        type("n", surface)
        type("y", surface)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertTrue(surface.statusBarState.message.contains("1 approved"))
        type("l", surface)
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertTrue(surface.canUndo)
        XCTAssertNotEqual(try backend.recoverySnapshot(), before)
        type("u", surface)
        XCTAssertEqual(try backend.recoverySnapshot().source, before.source)
        XCTAssertFalse(surface.canUndo)
    }

    func testNativeEscapeCommitsPriorApprovalsAndNoApprovalCancelsCleanly() throws {
        let (backend, surface) = try fixture("cat cat")
        let before = try backend.recoverySnapshot()
        try begin(":s/cat/dog/gc", surface)
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        try begin(":s/cat/dog/gc", surface)
        type("y", surface)
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertTrue(surface.canUndo)
        type("u", surface)
        XCTAssertEqual(try backend.recoverySnapshot().source, before.source)
    }
    func testOtherPaneEditRejectsStaleApprovalsWithoutChangingItsText() throws {
        let (backend, surface) = try fixture("cat cat")
        let other = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        other.loadViewIfNeeded()
        other.view.frame = NSRect(x: 0, y: 0, width: 640, height: 260)
        other.viewDidLayout()
        try begin(":s/cat/dog/gc", surface)
        type("y", surface)
        type("i!", other)
        other.performInput { _ = try XCTUnwrap(other.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        let changed = try backend.recoverySnapshot()
        type("a", surface)
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertTrue(surface.commandOutput?.contains("document changed") == true, surface.commandOutput ?? "No diagnostic")
        XCTAssertEqual(try backend.recoverySnapshot(), changed)
    }

}
