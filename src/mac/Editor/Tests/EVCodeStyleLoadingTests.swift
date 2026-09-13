import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVCodeStyleLoadingTests: XCTestCase {
    func testInvalidStyleDefinitionsIdentifyTheSettingsFileAtEmptyDocumentStartup() throws {
        let (configuration, file, invalid) = try invalidConfiguration()
        let backend = EVCoreDocumentBackend(configuration: configuration)
        XCTAssertEqual(try backend.formattedText(), "")
        let message = try XCTUnwrap(backend.configurationWarning)
        XCTAssertTrue(message.contains("Code stylesheet"))
        XCTAssertTrue(message.contains(file.path))
        XCTAssertTrue(message.contains("could not be loaded"))
        XCTAssertTrue(message.contains("invalid style definitions"))
        XCTAssertFalse(message.contains("style edit was rejected"))
        XCTAssertFalse(message.contains("core status"))

        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertEqual(session.lastError, message)
        let root = try XCTUnwrap(session.snapshot().definition(for: .baseParagraph))
        XCTAssertEqual(root.properties[.characterSize]?.effective, .float(14))
        XCTAssertEqual(root.properties[.characterFontFamilies]?.effective, .stringList(["monospace"]))
        XCTAssertFalse(try session.snapshot().definitions.contains { $0.key.id.rawValue == "Document" })
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(try Data(contentsOf: file), invalid)

        let another = EVCoreDocumentBackend(configuration: configuration)
        XCTAssertEqual(another.configurationWarning, message,
                       "The cached startup failure keeps the same file-specific explanation")
        XCTAssertEqual(try another.formattedText(), "")
        XCTAssertEqual(try Data(contentsOf: file), invalid)
    }

    func testExternalValidStylesClearTheCachedStartupFailureWithoutRewritingTheFile() async throws {
        let (configuration, file, invalid) = try invalidConfiguration()
        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertNotNil(session.lastError)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
        let valid = Data(#"{"version":2,"block_styles":[{"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","character":{"size":22},"block":{}}]}"#.utf8)
        try valid.write(to: file, options: .atomic)
        await withCheckedContinuation { continuation in
            EVCodeStyleSession.checkExternalStyleChanges { continuation.resume() }
        }

        XCTAssertNil(session.lastError)
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?
            .properties[.characterSize]?.effective, .float(22))
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertNoThrow(try EVCodeStyleSession.initialize(configuration: configuration))
        let backend = EVCoreDocumentBackend(configuration: configuration)
        XCTAssertNil(backend.configurationWarning)
        XCTAssertEqual(try backend.formattedText(), "")
        XCTAssertEqual(try Data(contentsOf: file), valid)
    }

    private func invalidConfiguration() throws -> (EVConfigurationStore, URL, Data) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-code-style-loading-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("code_style.json")
        let invalid = Data(#"{"version":2,"block_styles":[{"id":"Document","name":"Base Document","role":"Document","character":{"font_families":["Courier"],"size":31},"block":{}}]}"#.utf8)
        try invalid.write(to: file)
        return (configuration, file, invalid)
    }
}
