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
    private func perform(_ window: EVDocumentWindowController, backend: EVCoreDocumentBackend, kind: EVDocumentHostRequest.Kind, path: String? = nil, force: Bool = false, range: ClosedRange<UInt64>? = nil, after: UInt64? = nil) async -> Result<String?, Error> {
        let state = backend.persistenceState
        return await withCheckedContinuation { continuation in
            window.perform(documentHostRequests: [.init(kind: kind, documentID: state.documentID, documentRevision: state.documentRevision, force: force, path: path, hardLineRange: range, readAfterLine: after)]) { continuation.resume(returning: $0) }
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

    func testConvertedFormatWritesNewFileAndProtectsOriginalFromEveryExWrite() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); document.close(); try? FileManager.default.removeItem(at: directory) }
        let original = try XCTUnwrap(document.fileURL)
        let originalBytes = try backend.serializedSource(typeName: EVDocument.markdownType)
        _ = try await perform(window, backend: backend, kind: .write).get()
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        _ = try session.setFormat(.html, operation: .convert, expected: backend.documentState())
        let convertedBytes = try backend.serializedSource(typeName: EVDocument.htmlType)
        XCTAssertNotEqual(convertedBytes, originalBytes)
        XCTAssertEqual(try Data(contentsOf: original), originalBytes)
        XCTAssertTrue(document.requiresNewFormatDestination)

        for (kind, path) in [(EVDocumentHostRequest.Kind.write, Optional<String>.none), (.write, original.path), (.saveAs, original.path)] {
            let refused = await perform(window, backend: backend, kind: kind, path: path, force: true)
            switch refused {
            case .success: XCTFail("Changed serialization overwrote the original")
            case let .failure(error):
                XCTAssertEqual(error as? EVDocumentSerializationError, .changedFormatNeedsNewDestination)
            }
            XCTAssertEqual(try Data(contentsOf: original), originalBytes)
            XCTAssertEqual(document.fileURL, original)
        }

        let destination = directory.appendingPathComponent("converted.html")
        _ = try await perform(window, backend: backend, kind: .saveAs, path: destination.path).get()
        XCTAssertEqual(try Data(contentsOf: original), originalBytes)
        XCTAssertEqual(try Data(contentsOf: destination), convertedBytes)
        XCTAssertEqual(document.fileURL, EVDocumentIdentity.canonicalURL(destination))
        XCTAssertFalse(document.requiresNewFormatDestination)
        XCTAssertFalse(backend.persistenceState.isDirty)
        _ = try await perform(window, backend: backend, kind: .write).get()
        XCTAssertEqual(try Data(contentsOf: original), originalBytes)
        XCTAssertEqual(try Data(contentsOf: destination), convertedBytes)
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
    func testReadUsesActiveSharedViewAndIsOneUndoableChange() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("first\r\nlast".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let first = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let firstSession = try XCTUnwrap(first.session)
        _ = try await perform(window, backend: backend, kind: .split).get()
        let active = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(active.session)
        XCTAssertFalse(active === first)
        let input = directory.appendingPathComponent("read.txt")
        try Data("\u{feff}  one\r\n  two\r\n".utf8).write(to: input)
        _ = try await perform(window, backend: backend, kind: .read, path: input.path, after: 1).get()
        XCTAssertEqual(try backend.formattedText(), "first\n  one\n  two\nlast")
        XCTAssertEqual(try firstSession.presentation().cursor_utf8_offset, 0)
        XCTAssertEqual(try session.presentation().cursor_utf8_offset, 8)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data("first\r\n  one\r\n  two\r\nlast".utf8))
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UInt32(Character("u").asciiValue!))
        XCTAssertEqual(try backend.formattedText(), "first\nlast")
    }

    func testReadRejectsStaleCompletionAndHandlesEmptyBufferWithoutExtraLine() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data(), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        let stale = backend.persistenceState
        _ = try session.readFile(Data("one\ntwo\n".utf8), after: 1, expected: stale)
        XCTAssertEqual(try backend.formattedText(), "one\ntwo")
        XCTAssertThrowsError(try session.readFile(Data("bad".utf8), after: 1, expected: stale))
        XCTAssertEqual(try backend.formattedText(), "one\ntwo")
    }

    func testSourceRunsMappingsEditsNestedFilesAndStopsAtPathLineError() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("b\na".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let child = directory.appendingPathComponent("child.viem")
        try Data("map Y y$\nset hlsearch incsearch\n".utf8).write(to: child)
        let source = directory.appendingPathComponent("source.viem")
        try Data("\u{feff}\" comment\r\n%sort\nsource \(child.path)\n1delete\n".utf8).write(to: source)
        _ = try await perform(window, backend: backend, kind: .source, path: source.path).get()
        XCTAssertEqual(try backend.formattedText(), "b")
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        _ = try session.sendText("Y")
        XCTAssertEqual(try backend.formattedText(), "b", "sourced mapping yanks instead of inserting")
        let bad = directory.appendingPathComponent("bad.viem")
        try Data("set nowrap\nnotacommand\n1delete\n".utf8).write(to: bad)
        let result = await perform(window, backend: backend, kind: .source, path: bad.path)
        guard case .failure(let error) = result else { return XCTFail("Invalid sourced line accepted") }
        XCTAssertTrue(error.localizedDescription.contains("bad.viem:2:"), error.localizedDescription)
        XCTAssertEqual(try backend.formattedText(), "b")
    }

    func testSourceRejectsConfirmationAndRecursiveFilesWithoutLeavingPrompt() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("one one".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let source = directory.appendingPathComponent("confirm.viem")
        try Data("%s/one/two/gc\n".utf8).write(to: source)
        if case .success = await perform(window, backend: backend, kind: .source, path: source.path) { XCTFail("Interactive source accepted") }
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        surface.refreshPresentation()
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertEqual(try backend.formattedText(), "one one")
        try Data("source \(source.path)\n".utf8).write(to: source)
        let result = await perform(window, backend: backend, kind: .source, path: source.path)
        guard case .failure(let error) = result else { return XCTFail("Recursive source accepted") }
        XCTAssertTrue(error.localizedDescription.contains("nesting limit"), error.localizedDescription)
        XCTAssertEqual(try backend.formattedText(), "one one")
    }

    func testFileRenamesBindingWithoutWritesOrDirtyChangesAndPreservesOverwriteReview() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let original = try XCTUnwrap(document.fileURL)
        _ = try await perform(window, backend: backend, kind: .write).get()
        let bytes = try Data(contentsOf: original)
        let target = directory.appendingPathComponent("renamed.md")
        _ = try await perform(window, backend: backend, kind: .file, path: target.path).get()
        XCTAssertEqual(document.fileURL, target)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: target.path))
        XCTAssertEqual(try Data(contentsOf: original), bytes)
        _ = try await perform(window, backend: backend, kind: .write).get()
        XCTAssertEqual(try Data(contentsOf: target), bytes)
        XCTAssertEqual(try Data(contentsOf: original), bytes)
        let occupied = directory.appendingPathComponent("occupied.md")
        try Data("do not overwrite".utf8).write(to: occupied)
        _ = try await perform(window, backend: backend, kind: .file, path: occupied.path).get()
        document.externalSaveDecisionHandler = { _ in false }
        if case .success = await perform(window, backend: backend, kind: .write) { XCTFail("Renaming acknowledged unrelated existing bytes") }
        XCTAssertEqual(try Data(contentsOf: occupied), Data("do not overwrite".utf8))
        document.externalSaveDecisionHandler = { _ in true }
        _ = try await perform(window, backend: backend, kind: .write).get()
        XCTAssertEqual(try Data(contentsOf: occupied), bytes)
    }

    func testOnlyRefusesDirtyLastViewsAndBangKeepsActivePane() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let originalSurface = window.editorSurface
        let session = try XCTUnwrap((originalSurface as? EVEditorSurfaceController)?.session)
        _ = try session.sendText("iChanged")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        _ = try await perform(window, backend: backend, kind: .newPane).get()
        let active = try XCTUnwrap(window.activeDocument)
        let activeBackend = try XCTUnwrap(active.editorBackend as? EVCoreDocumentBackend)
        let kept = window.editorSurface
        if case .success = await perform(window, backend: activeBackend, kind: .only) { XCTFail("Dirty last view discarded") }
        XCTAssertEqual(window.paneCount, 2)
        _ = try await perform(window, backend: activeBackend, kind: .only, force: true).get()
        XCTAssertEqual(window.paneCount, 1)
        XCTAssertTrue(window.editorSurface === kept)
        XCTAssertTrue(window.activeDocument === active)
        XCTAssertFalse(window.activeDocument === document)
    }

    func testFilePreservesSameNameBaselineAndRefusesAnotherLiveDocument() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let original = try XCTUnwrap(document.fileURL)
        document.recordFileBaseline(Data("original".utf8), at: original)
        let baseline = document.fileBaseline
        _ = try await perform(window, backend: backend, kind: .file, path: original.path).get()
        XCTAssertEqual(document.fileBaseline, baseline)
        let target = directory.appendingPathComponent("owned.md")
        _ = try await perform(window, backend: backend, kind: .editNewWindow, path: target.path).get()
        let other = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: target))
        defer { for controller in other.windowControllers { controller.close() }; other.close() }
        if case .success = await perform(window, backend: backend, kind: .file, path: target.path) { XCTFail("A second document acquired the same file identity") }
        XCTAssertEqual(document.fileURL, original)
        XCTAssertEqual(document.fileBaseline, baseline)
    }

}
