import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVCodeStyleLoadingTests: XCTestCase {
    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-theme-loading-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory)
    }

    func testMalformedSelectedThemeFallsBackWithoutRewritingTheFile() throws {
        let original = configuration()
        let file = try XCTUnwrap(original.selectedThemeURL)
        let invalid = Data("invalid JSON".utf8)
        try invalid.write(to: file)
        let reopened = EVConfigurationStore(directory: original.directory)
        let backend = EVCoreDocumentBackend(configuration: reopened)
        XCTAssertEqual(try backend.formattedText(), "")
        XCTAssertNotNil(backend.configurationWarning)
        XCTAssertNil(reopened.currentThemeName)
        let session = try EVCodeStyleSession(configuration: reopened)
        XCTAssertFalse(try session.snapshot().definitions.isEmpty)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
    }

    func testRepairedThemeCanBeSelectedWithoutRewritingExternalContents() throws {
        let original = configuration()
        let file = try XCTUnwrap(original.selectedThemeURL)
        let valid = try Data(contentsOf: file)
        try Data("invalid JSON".utf8).write(to: file)
        let reopened = EVConfigurationStore(directory: original.directory)
        let session = try EVCodeStyleSession(configuration: reopened)
        XCTAssertNil(reopened.currentThemeName)
        try valid.write(to: file, options: .atomic)
        try reopened.selectTheme(named: "Midnight")
        XCTAssertEqual(reopened.currentThemeName, "Midnight")
        XCTAssertNil(session.lastError)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertEqual(try Data(contentsOf: file), valid)
        let backend = EVCoreDocumentBackend(configuration: reopened)
        XCTAssertNil(backend.configurationWarning)
    }
}
