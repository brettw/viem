import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor final class EVNativeExFileTests: XCTestCase {
    private func fixture() throws -> (URL, EVCoreDocumentBackend, EVDocument, EVDocumentWindowController) {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-ex-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let backend = EVCoreDocumentBackend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("# raw **source**\r\n".utf8), ofType: EVDocument.markdownType)
        document.fileURL = directory.appendingPathComponent("original.md")
        document.fileType = EVDocument.markdownType
        let window = EVDocumentWindowController(document: document, editorSurface: backend.makeEditorSurface())
        document.addWindowController(window)
        return (directory, backend, document, window)
    }
    private func perform(_ window: EVDocumentWindowController, backend: EVCoreDocumentBackend, kind: EVDocumentHostRequest.Kind, path: String? = nil, force: Bool = false, range: ClosedRange<UInt64>? = nil) async -> Result<String?, Error> {
        let state = backend.persistenceState
        return await withCheckedContinuation { continuation in
            window.perform(documentHostRequests: [.init(kind: kind, documentID: state.documentID, documentRevision: state.documentRevision, force: force, path: path, hardLineRange: range)]) { continuation.resume(returning: $0) }
        }
    }
    func testAlternateWritePreservesBytesBindingDirtyStateAndRejectsOverwriteAndReadOnly() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        _ = try session.sendText("iChanged ")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        let bytes = try backend.serializedSource(typeName: EVDocument.markdownType)
        let original = document.fileURL
        let alternate = directory.appendingPathComponent("copy.txt")
        _ = try await perform(window, backend: backend, kind: .write, path: alternate.path).get()
        XCTAssertEqual(try Data(contentsOf: alternate), bytes)
        XCTAssertEqual(document.fileURL, original)
        XCTAssertTrue(backend.persistenceState.isDirty)
        let conflict = await perform(window, backend: backend, kind: .write, path: alternate.path)
        if case .success = conflict { XCTFail("Existing alternate target was overwritten") }
        XCTAssertEqual(try Data(contentsOf: alternate), bytes)
        try backend.setReadOnly(true)
        let refused = directory.appendingPathComponent("readonly.md")
        let readOnly = await perform(window, backend: backend, kind: .write, path: refused.path)
        if case .success = readOnly { XCTFail("Read-only write was accepted without bang") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: refused.path))
        _ = try await perform(window, backend: backend, kind: .write, path: refused.path, force: true).get()
        XCTAssertEqual(try Data(contentsOf: refused), bytes)
    }
    func testRangedWriteCopiesPhysicalSyntaxAndSaveAsAdoptsOnlyAfterSuccess() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("# first\r\n**second**\r\nlast".utf8), ofType: EVDocument.markdownType)
        let target = directory.appendingPathComponent("range.md")
        _ = try await perform(window, backend: backend, kind: .write, path: target.path, range: 1...1).get()
        XCTAssertEqual(try Data(contentsOf: target), Data("**second**\r\nlast".utf8))
        let original = try XCTUnwrap(document.fileURL)
        let refusal = await perform(window, backend: backend, kind: .write, path: original.path, range: 1...1)
        if case .success = refusal { XCTFail("Partial overwrite did not require force") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: original.path))
        let conflictingSaveAs = await perform(window, backend: backend, kind: .saveAs, path: target.path)
        if case .success = conflictingSaveAs { XCTFail("Save As overwrote an existing file") }
        XCTAssertEqual(document.fileURL, original)
        let adopted = directory.appendingPathComponent("adopted.txt")
        _ = try await perform(window, backend: backend, kind: .saveAs, path: adopted.path).get()
        XCTAssertTrue(EVDocumentIdentity.sameFile(try XCTUnwrap(document.fileURL), adopted))
        XCTAssertEqual(document.fileType, EVDocument.markdownType, "a filename extension does not convert authoritative source")
        XCTAssertEqual(try Data(contentsOf: adopted), try backend.serializedSource(typeName: EVDocument.markdownType))
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testWorkingDirectoryIsSharedByPwdAndRelativeFileCommands() async throws {
        let previous = FileManager.default.currentDirectoryPath
        let (directory, backend, document, window) = try fixture()
        defer { _ = FileManager.default.changeCurrentDirectoryPath(previous); window.close(); try? FileManager.default.removeItem(at: directory) }
        _ = try await perform(window, backend: backend, kind: .changeDirectory, path: directory.path).get()
        let cwd = try await perform(window, backend: backend, kind: .printWorkingDirectory).get()
        XCTAssertTrue(EVDocumentIdentity.sameFile(URL(fileURLWithPath: try XCTUnwrap(cwd)), directory))
        _ = try await perform(window, backend: backend, kind: .write, path: "relative.md").get()
        XCTAssertEqual(try Data(contentsOf: directory.appendingPathComponent("relative.md")), try backend.serializedSource(typeName: EVDocument.markdownType))
        XCTAssertEqual(document.fileURL?.lastPathComponent, "original.md")
    }
    func testEditReplacesActivePaneWhileUppercaseEditAddsWindow() async throws {
        let (directory, backend, original, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let target = directory.appendingPathComponent("next.md")
        _ = try await perform(window, backend: backend, kind: .editNewWindow, path: target.path).get()
        XCTAssertTrue(window.activeDocument === original)
        let opened = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: target))
        XCTAssertEqual(opened.windowControllers.count, 1)
        _ = try await perform(window, backend: backend, kind: .edit, path: target.path).get()
        XCTAssertTrue(window.activeDocument === opened)
        XCTAssertTrue(window.document === opened)
        for controller in opened.windowControllers where controller !== window { controller.close() }
    }
}
