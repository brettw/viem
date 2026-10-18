import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor final class EVNativeExFileTests: XCTestCase {
    private func fixture() throws -> (URL, EVCoreDocumentBackend, EVDocument, EVDocumentWindowController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-ex-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("profile"))
        EVFrontendRegistry.install { EVCoreDocumentBackend(configuration: configuration) }
        addTeardownBlock { @MainActor in EVFrontendRegistry.install { EVCoreDocumentBackend() } }
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("# raw **source**\r\n".utf8), ofType: EVDocument.markdownType)
        document.fileURL = directory.appendingPathComponent("original.md")
        document.fileType = EVDocument.markdownType
        let window = EVDocumentWindowController(document: document, editorSurface: backend.makeEditorSurface())
        document.addWindowController(window)
        return (directory, backend, document, window)
    }
    private func perform(_ window: EVDocumentWindowController, backend: EVCoreDocumentBackend, kind: EVDocumentHostRequest.Kind, path: String? = nil, force: Bool = false, range: Range<UInt64>? = nil, after: UInt64? = nil) async -> Result<String?, Error> {
        let state = backend.persistenceState
        return await withCheckedContinuation { continuation in
            window.perform(documentHostRequests: [.init(kind: kind, documentID: state.documentID, documentRevision: state.documentRevision, force: force, path: path, sourceByteRange: range, readAfterOffset: after, readAfterIsSource: true)]) { continuation.resume(returning: $0) }
        }
    }
    func testFilenameDirectoryMenuUsesNativeCdAndPreservesDocumentState() throws {
        let (directory, backend, document, window) = try fixture()
        let previous = FileManager.default.currentDirectoryPath
        defer { _ = FileManager.default.changeCurrentDirectoryPath(previous); window.close(); try? FileManager.default.removeItem(at: directory) }
        let destination = directory.appendingPathComponent("folder with | café #")
        try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
        window.showWindow(nil)
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        let before = try session.presentation()
        let source = try backend.serializedSource(typeName: EVDocument.markdownType)
        func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
        let bar = try XCTUnwrap(descendants(try XCTUnwrap(window.window?.contentView)).compactMap { $0 as? EVStatusBarView }.first)
        let menu = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }.first { $0.accessibilityLabel()?.hasPrefix("File: ") == true }?.menu)
        // Invoke an already-created menu after the document is rebound.
        document.fileURL = destination.appendingPathComponent("renamed.md")
        menu.update()
        XCTAssertTrue(menu.items.last?.isEnabled == true)
        menu.performActionForItem(at: menu.numberOfItems - 1)
        XCTAssertTrue(EVDocumentIdentity.sameFile(URL(fileURLWithPath: FileManager.default.currentDirectoryPath), destination))
        XCTAssertEqual(bar.filePath, "renamed.md")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(try session.presentation().mode, before.mode)
        XCTAssertEqual(try session.presentation().cursor_utf8_offset, before.cursor_utf8_offset)
    }
    func testNewMarkdownExOpensUsePreferenceButCurrentFileReloadKeepsItsView() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let configuration = backend.configuration
        try configuration.setMarkdownFormattedView(false)
        let original = try XCTUnwrap(document.fileURL)
        try Data("# Reloaded".utf8).write(to: original)
        _ = try await perform(window, backend: backend, kind: .edit).get()
        XCTAssertEqual(backend.sourceFormat, .markdown, "Reload retains this document's formatted view")
        XCTAssertFalse(configuration.markdownFormattedView, "Reload does not change another document's newer choice")

        for enabled in [true, false] {
            try configuration.setMarkdownFormattedView(enabled)
            let target = directory.appendingPathComponent("target-\(enabled).md")
            let activeBackend = try XCTUnwrap(window.activeDocument?.editorBackend as? EVCoreDocumentBackend)
            _ = try await perform(window, backend: activeBackend, kind: .edit, path: target.path).get()
            let opened = try XCTUnwrap(window.activeDocument)
            XCTAssertEqual(opened.editorBackend.sourceFormat, enabled ? .markdown : .code)
            XCTAssertFalse(opened.editorBackend.persistenceState.isDirty)
        }
    }

    func testAlternateWritePreservesBytesBindingDirtyStateAndRejectsOverwriteAndReadOnly() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        _ = try session.sendTestKeys("iChanged ")
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
        _ = try await perform(window, backend: backend, kind: .write, path: target.path, range: 9..<21).get()
        XCTAssertEqual(try Data(contentsOf: target), Data("**second**\r\n".utf8))
        let original = try XCTUnwrap(document.fileURL)
        let refusal = await perform(window, backend: backend, kind: .write, path: original.path, range: 9..<21)
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
        _ = try await perform(window, backend: backend, kind: .read, path: input.path, after: 7).get()
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
        _ = try session.readFile(Data("one\ntwo\n".utf8), after: 0, source: true, expected: stale)
        XCTAssertEqual(try backend.formattedText(), "one\ntwo")
        XCTAssertThrowsError(try session.readFile(Data("bad".utf8), after: 0, source: true, expected: stale))
        XCTAssertEqual(try backend.formattedText(), "one\ntwo")
    }

    func testSourceRunsScriptsWithMappingsNestedFilesAndContinuesAfterErrors() throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("b\na".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        let child = directory.appendingPathComponent("child.viem")
        try Data("map Y y$\nset hlsearch incsearch\n".utf8).write(to: child)
        let source = directory.appendingPathComponent("source.viem")
        try Data("\u{feff}\" comment\r\n%sort\nsource \(child.path)\n1delete\nlet g:done = 1\n".utf8).write(to: source)
        let outcome = try session.executeCommandLine("source \(source.path)")
        XCTAssertEqual(outcome.command_status, UInt32(VIEM_COMMAND_STATUS_COMPLETE))
        XCTAssertEqual(try backend.formattedText(), "b")
        _ = try session.sendText("Y")
        XCTAssertEqual(try backend.formattedText(), "b", "sourced mapping yanks instead of inserting")
        // As in Vim, an error reports its file and line, and the script
        // continues with the next line.
        let bad = directory.appendingPathComponent("bad.viem")
        try Data("set nowrap\nnotacommand\n1delete\n".utf8).write(to: bad)
        let failed = try session.executeCommandLine("source \(bad.path)")
        XCTAssertEqual(failed.command_status, UInt32(VIEM_COMMAND_STATUS_ERROR))
        let message = try XCTUnwrap(surface.commandOutput)
        XCTAssertTrue(message.contains("bad.viem:2: E492: Not an editor command: notacommand"), message)
        XCTAssertEqual(try backend.formattedText(), "")
    }

    func testSourceWaitsForNativeReadBeforeRunningNextLine() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("tail".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        let imported = directory.appendingPathComponent("import.txt")
        try Data("head\n".utf8).write(to: imported)
        let source = directory.appendingPathComponent("read.viem")
        try Data("0read \(imported.path)\ncall setline(2, 'after')\necho 'finished'\n".utf8).write(to: source)
        _ = try session.executeCommandLine("source \(source.path)")
        let deadline = Date().addingTimeInterval(2)
        while surface.commandOutput != "finished", Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(surface.commandOutput, "finished")
        XCTAssertEqual(try backend.formattedText(), "head\nafter")
    }

    func testFailedNativeEffectCancelsRemainingSourceCommands() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("original".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        _ = try session.executeCommandLine("call setline(1, 'prefix') | read \(directory.appendingPathComponent("missing.txt").path) | call setline(1, 'never')")
        // The error path schedules acknowledgement on the main queue.
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(try backend.formattedText(), "prefix")
        _ = try session.executeCommandLine("echo 'ready'")
        XCTAssertEqual(surface.commandOutput, "ready")
    }

    func testQueuedScriptCancellationKeepsBorrowedDocumentAliveAfterSurfaceCloses() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-continuation-lifetime-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        weak var releasedBackend: EVCoreDocumentBackend?
        weak var releasedSurface: EVEditorSurfaceController?
        weak var releasedSession: EVCoreViewSession?
        try autoreleasepool {
            let configuration = EVConfigurationStore(directory: directory)
            let backend = EVCoreDocumentBackend(configuration: configuration)
            let document = EVDocument(editorBackend: backend)
            try document.read(from: Data("text".utf8), ofType: EVDocument.plainTextType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            let session = try XCTUnwrap(surface.session)
            releasedBackend = backend; releasedSurface = surface; releasedSession = session
            // There is no native host. Rejection queues a cancellation, then
            // every normal owner disappears before the main queue can run it.
            XCTAssertThrowsError(try session.executeCommandLine("pwd | let g:unreachable = 1"))
        }
        XCTAssertNil(releasedSurface)
        XCTAssertNotNil(releasedBackend, "the queued acknowledgement owns the borrowed document")
        let deadline = Date().addingTimeInterval(2)
        while releasedBackend != nil, Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertNil(releasedSession)
        XCTAssertNil(releasedBackend, "the acknowledgement must release its temporary owner")
    }

    func testSourceRejectsConfirmationAndRecursiveFilesWithoutLeavingPrompt() throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("one one".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let surface = try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        let source = directory.appendingPathComponent("confirm.viem")
        try Data("%s/one/two/gc\n".utf8).write(to: source)
        let confirmation = try session.executeCommandLine("source \(source.path)")
        XCTAssertEqual(confirmation.command_status, UInt32(VIEM_COMMAND_STATUS_ERROR))
        surface.refreshPresentation()
        XCTAssertNil(surface.substituteConfirmationPrompt)
        XCTAssertEqual(try backend.formattedText(), "one one")
        try Data("source \(source.path)\n".utf8).write(to: source)
        let recursive = try session.executeCommandLine("source \(source.path)")
        XCTAssertEqual(recursive.command_status, UInt32(VIEM_COMMAND_STATUS_ERROR))
        XCTAssertTrue(surface.commandOutput?.contains("source nesting limit exceeded") == true, surface.commandOutput ?? "")
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
        var reviews = 0
        document.externalSaveDecisionHandler = { _ in reviews += 1; return false }
        // Keep cancellation distinct from success so a suspended script can
        // abandon its remaining commands. The surface suppresses its message.
        let cancelled = await perform(window, backend: backend, kind: .write)
        if case .failure(let error) = cancelled {
            XCTAssertEqual((error as? CocoaError)?.code, .userCancelled)
        } else { XCTFail("cancelled save must not acknowledge script success") }
        XCTAssertEqual(reviews, 1)
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
        _ = try session.sendTestKeys("iChanged")
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

    func testSaveAutocommandsWritePreEditsAndKeepPostEditsOpenOnWriteQuit() async throws {
        let (directory, backend, document, window) = try fixture()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try document.read(from: Data("original".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        let target = directory.appendingPathComponent("saved.txt")
        document.fileURL = target
        let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
        _ = try session.executeCommandLine("autocmd BufWritePre * call setline(1, 'saved')")
        _ = try session.executeCommandLine("autocmd BufWritePost * call setline(1, 'after')")
        let result = await perform(window, backend: backend, kind: .writeQuit)
        if case .success = result { XCTFail("BufWritePost edits must keep the document open") }
        XCTAssertEqual(try Data(contentsOf: target), Data("saved".utf8))
        XCTAssertEqual(try backend.formattedText(), "after")
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertTrue(window.activeDocument === document)
    }

}
