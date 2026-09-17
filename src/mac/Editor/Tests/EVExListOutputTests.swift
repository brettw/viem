import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVExListOutputTests: XCTestCase {
    func testListEscapesLiteralControlsWhileNumberKeepsRawLogicalContent() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-list-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        let first = "A\tB\0\u{1}\n\u{1b}\u{1f}\u{7f}\u{85}"
        let source = Data((first + "\rcafé 👩‍💻\r").utf8)
        try backend.restoreRecovery(EVRecoverySnapshot(source: source, format: .plainText,
            encoding: UInt32(VIEM_ENCODING_UTF8), fileFormat: UInt32(VIEM_FILE_FORMAT_MAC),
            documentID: 1, documentRevision: 1))
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 640, height: 260)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        func command(_ text: String) throws {
            surface.performInput {
                _ = try session.sendText(text)
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
            }
        }
        let before = try backend.recoverySnapshot()
        try command(":1,2l")
        XCTAssertEqual(surface.commandOutput, "A^IB^@^A^J^[^_^?<85>$\ncafé 👩‍💻$")
        try command(":1,2nu")
        XCTAssertEqual(surface.commandOutput, "1\t\(first)\n2\tcafé 👩‍💻")
        try command(":1,2nu l")
        XCTAssertEqual(surface.commandOutput, "1\tA^IB^@^A^J^[^_^?<85>$\n2\tcafé 👩‍💻$")
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)

        // Unix interpretation leaves an interior CR as literal line content.
        let unixBackend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try unixBackend.restoreRecovery(EVRecoverySnapshot(source: Data("A\rB\n".utf8), format: .plainText,
            encoding: UInt32(VIEM_ENCODING_UTF8), fileFormat: UInt32(VIEM_FILE_FORMAT_UNIX),
            documentID: 2, documentRevision: 1))
        let unixSurface = try XCTUnwrap(unixBackend.makeEditorSurface() as? EVEditorSurfaceController)
        unixSurface.loadViewIfNeeded()
        let unixSession = try XCTUnwrap(unixSurface.session)
        unixSurface.performInput {
            _ = try unixSession.sendText(":1l")
            _ = try unixSession.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertEqual(unixSurface.commandOutput, "A^MB$")
    }
}
