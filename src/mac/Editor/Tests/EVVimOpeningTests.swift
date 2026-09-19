import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVVimOpeningTests: XCTestCase {
    private func checkVimrcPaint(configuration: EVConfigurationStore, directory: URL, command: String = "set") async throws {
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
        XCTAssertTrue(highlighted, "Configured vim.vim did not paint \(command) from \(configuration.bundledVimSyntaxDirectory)")
        XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
        XCTAssertFalse(document.isDocumentEdited)
    }

    private func bundledConfiguration(directory: URL, syntaxFile: String) throws -> EVConfigurationStore {
        // SwiftPM's test bundle does not contain application resources. Inject
        // the packaged app explicitly and fail if packaging omitted the runtime.
        var checkout = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { checkout.deleteLastPathComponent() }
        let resources = checkout.appendingPathComponent(".build/Viem.app/Contents/Resources")
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"),
          legacyDefaults: nil, bundleResourceURL: resources)
        let syntax = URL(fileURLWithPath: configuration.bundledVimSyntaxDirectory).appendingPathComponent(syntaxFile)
        _ = try Data(contentsOf: syntax)
        return configuration
    }

    private func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-vimrc-syntax-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testVimrcUsesTheBundledResourceResolver() async throws {
        let directory = try directory()
        let resources = directory.appendingPathComponent("Resources")
        let syntaxDirectory = resources.appendingPathComponent("vim/runtime/syntax")
        try FileManager.default.createDirectory(at: syntaxDirectory, withIntermediateDirectories: true)
        // An injected bundle fixture proves that the resource resolver supplies
        // the paint, instead of Code mode alone or another syntax provider.
        try Data("syn keyword viemVimFixture viemFixtureCommand\nhi def link viemVimFixture Statement\n".utf8)
            .write(to: syntaxDirectory.appendingPathComponent("vim.vim"))
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"),
          legacyDefaults: nil, bundleResourceURL: resources)
        try await checkVimrcPaint(configuration: configuration, directory: directory, command: "viemFixtureCommand")
    }

    func testBundledVimRuntimePaintsVimrcByDefault() async throws {
        let directory = try directory()
        let configuration = try bundledConfiguration(directory: directory, syntaxFile: "vim.vim")
        try await checkVimrcPaint(configuration: configuration, directory: directory)
    }

    func testBundledMakefileSyntaxOpensAndPaintsWithoutChangingSource() async throws {
        let directory = try directory()
        let configuration = try bundledConfiguration(directory: directory, syntaxFile: "make.vim")
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
        XCTAssertTrue(paintedTarget, "Bundled make.vim did not paint the target from \(configuration.bundledVimSyntaxDirectory)")
        XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testDocumentFullPathAndSaveAsUpdateVimSetupWithoutEditingSource() async throws {
        let directory = try directory()
        let resources = directory.appendingPathComponent("Resources")
        let syntaxDirectory = resources.appendingPathComponent("vim/runtime/syntax")
        try FileManager.default.createDirectory(at: syntaxDirectory, withIntermediateDirectories: true)
        let original = EVDocumentIdentity.canonicalURL(directory.appendingPathComponent("original.vim"))
        let renamed = directory.appendingPathComponent("renamed.vim")
        let quotedPath = original.path.replacingOccurrences(of: "'", with: "''")
        let syntax = "if expand('%') ==# '\(quotedPath)'\n  syn keyword Comment token\nelse\n  syn keyword Constant token\nendif\n"
        try Data(syntax.utf8).write(to: syntaxDirectory.appendingPathComponent("vim.vim"))
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("settings"),
          legacyDefaults: nil, bundleResourceURL: resources)
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
