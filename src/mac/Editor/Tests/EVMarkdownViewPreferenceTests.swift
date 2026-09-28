import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVMarkdownViewPreferenceTests: XCTestCase {
    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-markdown-view-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory)
    }

    private func surface(configuration: EVConfigurationStore, type: String? = nil)
        throws -> EVEditorSurfaceController {
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("# Heading\n\n**Words**".utf8), typeName: type ?? EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
    }

    func testNativeMarkdownOpensUseRememberedChoiceAcrossConfigurationReloads() throws {
        let configuration = configuration()
        for enabled in [false, true, false] {
            if enabled || configuration.markdownFormattedView { try configuration.setMarkdownFormattedView(enabled) }
            let reopened = EVConfigurationStore(directory: configuration.directory)
            for (name, nativeType) in [("notes.md", "public.data"), ("typed-markdown", EVDocument.markdownType)] {
                let url = configuration.directory.appendingPathComponent(name)
                let source = Data("# Heading\r\n".utf8)
                try source.write(to: url)
                let backend = EVCoreDocumentBackend(configuration: reopened)
                let document = EVDocument(editorBackend: backend)
                document.recordRecentDocument = { _ in }
                defer { document.close() }
                try document.read(from: url, ofType: nativeType)
                XCTAssertEqual(backend.sourceFormat, enabled ? .markdown : .markdownSource)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
                XCTAssertFalse(backend.persistenceState.isDirty)
            }
        }
    }

    func testToggleAndHistoryRememberActualTransitionsWithoutBackgroundViewsOverwritingThem() throws {
        let configuration = configuration()
        let first = try surface(configuration: configuration)
        let background = try surface(configuration: configuration)
        XCTAssertFalse(configuration.markdownFormattedView)
        first.formattingToolbar.formattedView.performClick(nil)
        XCTAssertEqual(first.backend.sourceFormat, .markdown)
        XCTAssertTrue(configuration.markdownFormattedView)
        XCTAssertTrue(EVConfigurationStore(directory: configuration.directory).markdownFormattedView)

        background.refreshPresentation()
        _ = try background.backend.documentState()
        let anotherSource = try surface(configuration: configuration)
        XCTAssertEqual(anotherSource.backend.sourceFormat, .markdownSource)
        XCTAssertTrue(configuration.markdownFormattedView, "Opening an explicit source specimen must not reset the default")
        first.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(first.backend.sourceFormat, .markdownSource)
        XCTAssertFalse(configuration.markdownFormattedView)
        first.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(first.backend.sourceFormat, .markdown)
        XCTAssertTrue(configuration.markdownFormattedView)

        for type in [EVDocument.plainTextType, EVDocument.codeType] {
            let other = try surface(configuration: configuration, type: type)
            other.setFormattedView(false)
            other.refreshPresentation()
            XCTAssertTrue(configuration.markdownFormattedView)
        }
    }

    func testExplicitRecoveryAndReloadKeepTheirViewWithoutChangingPreference() throws {
        let configuration = configuration()
        try configuration.setMarkdownFormattedView(true)
        let surface = try surface(configuration: configuration)
        let snapshot = try surface.backend.recoverySnapshot()
        try surface.backend.restoreRecovery(snapshot)
        XCTAssertEqual(surface.backend.sourceFormat, .markdownSource)
        XCTAssertTrue(configuration.markdownFormattedView)

        let url = configuration.directory.appendingPathComponent("reloaded.md")
        try Data("# Updated".utf8).write(to: url)
        let document = EVDocument(editorBackend: surface.backend)
        document.recordRecentDocument = { _ in }
        document.fileURL = url
        defer { document.close() }
        try document.installExternalFileSnapshot(EVExternalFileSnapshot.read(url), from: url)
        XCTAssertEqual(surface.backend.sourceFormat, .markdownSource)
        XCTAssertTrue(configuration.markdownFormattedView)
    }

    func testRejectedSwitchDoesNotRememberRequestedView() throws {
        let configuration = configuration()
        let surface = try surface(configuration: configuration)
        let session = try XCTUnwrap(surface.session)
        let stale = try surface.backend.documentState()
        _ = try session.sendText("iChange")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        XCTAssertThrowsError(try session.setMarkdownSource(false, expected: stale))
        XCTAssertEqual(surface.backend.sourceFormat, .markdownSource)
        XCTAssertFalse(configuration.markdownFormattedView)
    }

    func testPersistenceFailureReportsWarningWithoutUndoingSuccessfulViewChange() throws {
        let configuration = configuration()
        let surface = try surface(configuration: configuration)
        let file = configuration.directory.appendingPathComponent("config.json")
        let invalid = Data(#"{"version":2}"#.utf8)
        try invalid.write(to: file)
        surface.formattingToolbar.formattedView.performClick(nil)
        XCTAssertEqual(surface.backend.sourceFormat, .markdown)
        XCTAssertFalse(configuration.markdownFormattedView)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
        XCTAssertTrue(surface.commandOutput?.contains("Could not remember Markdown view") == true)
    }
}
