import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVVimOpeningTests: XCTestCase {
    private func checkVimrcPaint(syntaxDirectory: URL, directory: URL, command: String = "set") async throws {
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"), legacyDefaults: nil)
        try configuration.setVimSyntaxDirectory(syntaxDirectory.path)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        defer { document.close() }
        let url = directory.appendingPathComponent(".vimrc")
        let source = Data("\(command) number\r\nset expandtab\r\n\" editor settings\r\nlet g:example = 'value'\r\n".utf8)
        try source.write(to: url)
        try document.read(from: url, ofType: "public.data")
        XCTAssertEqual(backend.sourceFormat, .code)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        var highlighted = false
        for _ in 0..<500 {
            backend.pollSyntax()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= command.utf8.count
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true {
                highlighted = true
                break
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(highlighted, "Configured vim.vim did not paint \(command): \(EVCodePreferences.shared.loadDiagnostics)")
        XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
        XCTAssertFalse(document.isDocumentEdited)
    }

    private func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-vimrc-syntax-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testVimrcUsesTheConfiguredVimSyntaxFile() async throws {
        let directory = try directory()
        let syntaxDirectory = directory.appendingPathComponent("syntax")
        try FileManager.default.createDirectory(at: syntaxDirectory, withIntermediateDirectories: true)
        // A custom declaration proves that the configured Vim file supplies
        // the paint, instead of Code mode alone or another syntax provider.
        try Data("syn keyword viemVimFixture viemFixtureCommand\nhi def link viemVimFixture Statement\n".utf8)
            .write(to: syntaxDirectory.appendingPathComponent("vim.vim"))
        try await checkVimrcPaint(syntaxDirectory: syntaxDirectory, directory: directory, command: "viemFixtureCommand")
    }

    func testInstalledVimRuntimePaintsVimrc() async throws {
        let syntaxDirectory = URL(fileURLWithPath: EVCodePreferences.defaultVimSyntaxDirectory)
        guard FileManager.default.fileExists(atPath: syntaxDirectory.appendingPathComponent("vim.vim").path) else {
            throw XCTSkip("Pinned MacVim syntax runtime is not installed")
        }
        try await checkVimrcPaint(syntaxDirectory: syntaxDirectory, directory: directory())
    }

    func testInstalledMakefileSyntaxOpensAndPaintsWithoutChangingSource() async throws {
        let syntaxDirectory = URL(fileURLWithPath: EVCodePreferences.defaultVimSyntaxDirectory)
        guard FileManager.default.fileExists(atPath: syntaxDirectory.appendingPathComponent("make.vim").path) else {
            throw XCTSkip("Pinned MacVim syntax runtime is not installed")
        }
        let directory = try directory()
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"), legacyDefaults: nil)
        try configuration.setVimSyntaxDirectory(syntaxDirectory.path)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        defer { document.close() }
        let source = Data("all: input.txt\r\n\t@echo $(MESSAGE)\r\n".utf8)
        let url = directory.appendingPathComponent("Makefile")
        try source.write(to: url)
        try document.read(from: url, ofType: EVDocument.plainTextType)
        XCTAssertEqual(backend.sourceFormat, .code)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        var paintedTarget = false
        for _ in 0..<500 {
            backend.pollSyntax()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= 3
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true {
                paintedTarget = true
                break
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(paintedTarget, "Installed make.vim did not paint the target: \(EVCodePreferences.shared.loadDiagnostics)")
        XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testDocumentFullPathAndSaveAsUpdateVimSetupWithoutEditingSource() async throws {
        let directory = try directory()
        let syntaxDirectory = directory.appendingPathComponent("syntax")
        try FileManager.default.createDirectory(at: syntaxDirectory, withIntermediateDirectories: true)
        let original = EVDocumentIdentity.canonicalURL(directory.appendingPathComponent("original.vim"))
        let renamed = directory.appendingPathComponent("renamed.vim")
        let quotedPath = original.path.replacingOccurrences(of: "'", with: "''")
        let syntax = "if expand('%') ==# '\(quotedPath)'\n  syn keyword Comment token\nelse\n  syn keyword Constant token\nendif\n"
        try Data(syntax.utf8).write(to: syntaxDirectory.appendingPathComponent("vim.vim"))
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"), legacyDefaults: nil)
        try configuration.setVimSyntaxDirectory(syntaxDirectory.path)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        defer { document.close() }
        let source = Data("token\n".utf8)
        try source.write(to: original)
        try document.read(from: original, ofType: EVDocument.plainTextType)
        document.fileURL = original
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        func waitForStyle(_ name: String) async throws {
            for _ in 0..<200 {
                backend.pollSyntax()
                if try backend.syntaxStyleNames().contains(name) { return }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTFail("Filename-dependent Vim setup did not publish \(name)")
        }
        try await waitForStyle("Comment")
        let revision = backend.persistenceState.documentRevision
        let saved = expectation(description: "Save As updates syntax filename")
        document.save(to: renamed, ofType: EVDocument.plainTextType, for: .saveAsOperation) { error in
            XCTAssertNil(error)
            saved.fulfill()
        }
        await fulfillment(of: [saved], timeout: 3)
        XCTAssertEqual(document.fileURL, renamed)
        try await waitForStyle("Constant")
        XCTAssertFalse(try backend.syntaxStyleNames().contains("Comment"))
        XCTAssertEqual(backend.persistenceState.documentRevision, revision)
        XCTAssertEqual(try Data(contentsOf: original), source)
        XCTAssertEqual(try Data(contentsOf: renamed), source)
        XCTAssertFalse(document.isDocumentEdited)
    }
}
