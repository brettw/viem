import AppKit
import UniformTypeIdentifiers
import XCTest
@testable import ViemAppShell

@MainActor
final class EVDocumentExportTests: XCTestCase {
    private final class Surface: EVEditorSurface {
        let viewController = NSViewController()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
        var exportCount = 0
        var data = Data("<!doctype html><p>Styled copy</p>".utf8)
        func perform(menuCommand: EVMenuCommand, sender: Any?) {}
        func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
        func htmlExportData() async throws -> Data { exportCount += 1; return data }
    }

    private final class Backend: EVDocumentBackend {
        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState = EVDocumentPersistenceState(isDirty: true, documentID: 4, documentRevision: 12)
        var sourceFormat: EVSourceFormat = .markdownSource
        var source = Data("# Original\r\n".utf8)
        var acknowledgementCount = 0
        func makeEditorSurface() -> any EVEditorSurface { Surface() }
        func read(source: Data, typeName: String) throws { self.source = source }
        func serializedSource(typeName: String) throws -> Data { source }
        func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
            .init(data: source, documentID: 4, documentRevision: 12)
        }
        func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws { acknowledgementCount += 1 }
    }

    private func fixture(filename: String = "notes.md") throws -> (EVDocument, Backend, URL) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-export-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = Backend()
        let original = directory.appendingPathComponent(filename)
        try backend.source.write(to: original)
        let document = EVDocument(editorBackend: backend)
        document.fileURL = original
        document.fileType = EVDocument.markdownType
        document.recordFileBaseline(backend.source, at: original)
        return (document, backend, original)
    }

    func testExportPanelRestrictsHTMLAndSuggestsSafeCopyNames() throws {
        for (filename, expected) in [("notes.md", "notes.html"), ("page.HTML", "page-export.html"), ("page.xhtml", "page-export.html")] {
            let (document, _, original) = try fixture(filename: filename)
            defer { document.close() }
            let panel = document.makeHTMLExportPanel()
            XCTAssertEqual(panel.allowedContentTypes, [.html])
            XCTAssertFalse(panel.allowsOtherFileTypes)
            XCTAssertEqual(panel.nameFieldStringValue, expected)
            XCTAssertEqual(panel.directoryURL, original.deletingLastPathComponent())
            XCTAssertEqual(panel.prompt, "Export")
        }
    }

    func testCancelDoesNotGenerateOrWriteAFile() throws {
        let (document, backend, original) = try fixture()
        defer { document.close() }
        let surface = Surface()
        var shown = false
        document.htmlExportPanelHandler = { panel, complete in
            shown = true
            XCTAssertEqual(panel.allowedContentTypes, [.html])
            complete(nil)
        }
        document.exportHTML(from: surface)
        XCTAssertTrue(shown)
        XCTAssertEqual(surface.exportCount, 0)
        XCTAssertEqual(try Data(contentsOf: original), backend.source)
        XCTAssertEqual(backend.acknowledgementCount, 0)
    }

    func testExportWritesStyledCopyWithoutChangingDocumentOrSaveBaseline() async throws {
        let (document, backend, original) = try fixture()
        defer { document.close() }
        let destination = original.deletingPathExtension().appendingPathExtension("html")
        let surface = Surface()
        let before = backend.persistenceState
        let baseline = document.fileBaseline
        let baselineGeneration = document.fileBaselineGeneration
        let edited = document.isDocumentEdited
        document.htmlExportPanelHandler = { _, complete in complete(destination) }
        document.exportHTML(from: surface)
        for _ in 0..<200 where !FileManager.default.fileExists(atPath: destination.path) {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(try Data(contentsOf: destination), surface.data)
        XCTAssertEqual(surface.exportCount, 1)
        XCTAssertEqual(try Data(contentsOf: original), backend.source)
        XCTAssertEqual(document.fileURL, original)
        XCTAssertEqual(document.fileType, EVDocument.markdownType)
        XCTAssertEqual(backend.sourceFormat, .markdownSource)
        XCTAssertEqual(backend.persistenceState, before)
        XCTAssertEqual(document.isDocumentEdited, edited)
        XCTAssertEqual(document.fileBaseline, baseline)
        XCTAssertEqual(document.fileBaselineGeneration, baselineGeneration)
        XCTAssertEqual(backend.acknowledgementCount, 0)
    }

    func testExportRejectsCurrentSourceAliasesAndOtherOpenSources() async throws {
        let (document, backend, original) = try fixture()
        let (other, otherBackend, otherOriginal) = try fixture(filename: "other.html")
        NSDocumentController.shared.addDocument(other)
        defer {
            NSDocumentController.shared.removeDocument(other)
            document.close(); other.close()
        }
        let alias = original.deletingLastPathComponent().appendingPathComponent("alias.html")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: original)
        let hardLink = original.deletingLastPathComponent().appendingPathComponent("hardlink.html")
        try FileManager.default.linkItem(at: original, to: hardLink)
        for destination in [original, alias, hardLink, otherOriginal] {
            do {
                try await document.writeHTMLExport(Data("export".utf8), to: destination)
                XCTFail("Export must preserve open source files")
            } catch {
                XCTAssertEqual(error as? EVDocumentSerializationError, .exportOverwritesSource)
            }
        }
        XCTAssertEqual(try Data(contentsOf: original), backend.source)
        XCTAssertEqual(try Data(contentsOf: otherOriginal), otherBackend.source)
    }

    func testFailedExportKeepsSourceAndDirtyState() async throws {
        let (document, backend, original) = try fixture()
        defer { document.close() }
        let before = backend.persistenceState
        let destination = original.deletingLastPathComponent().appendingPathComponent("missing/page.html")
        do {
            try await document.writeHTMLExport(Data("export".utf8), to: destination)
            XCTFail("A missing parent directory must fail")
        } catch {}
        XCTAssertEqual(backend.persistenceState, before)
        XCTAssertEqual(document.fileURL, original)
        XCTAssertEqual(try Data(contentsOf: original), backend.source)
        XCTAssertEqual(backend.acknowledgementCount, 0)
    }
}
