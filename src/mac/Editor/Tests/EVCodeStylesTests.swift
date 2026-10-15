import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVCodeStylesTests: XCTestCase {
    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-style-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    func testUnknownCodeThemeExtensionsPreserveContinuousEditUndo() throws {
        let configuration = configuration()
        var code = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(configuration.codeStyleSheet())) as? [String: Any])
        code["futureExtension"] = ["enabled": true]
        try configuration.saveCodeStyleSheet(JSONSerialization.data(withJSONObject: code))
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
        ])
        let session = try EVCodeStyleSession(configuration: configuration)
        try session.beginGroup()
        for size: Float in [21, 25] {
            try session.edit(key: .baseParagraph, expected: session.snapshot().identity,
                mutation: .setDeclaration(.characterSize, .float(size)))
            XCTAssertTrue(session.isEditingGroup)
        }
        session.endGroup()
        session.undoManager.undo()
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(14))
        XCTAssertFalse(session.undoManager.canUndo)
        session.undoManager.redo()
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(25))
        let saved = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(configuration.codeStyleSheet())) as? [String: Any])
        XCTAssertEqual(saved["futureExtension"] as? [String: Bool], ["enabled": true])
    }

    func testNamesAreExactAndDeletionAndRenamePersistWithoutResurrectingBuiltins() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let initial = try session.snapshot()
        let keyword = try XCTUnwrap(initial.definitions.first { $0.name == "Keyword" })
        try session.edit(key: keyword.key, expected: initial.identity, mutation: .setDisplayName("KEYword"))
        let custom = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: UUID().uuidString.lowercased()))
        try session.create(key: custom, name: "Custom", expected: session.snapshot().identity)
        XCTAssertThrowsError(try session.edit(key: custom, expected: session.snapshot().identity,
            mutation: .setDisplayName("KEYword")), "Exact duplicate names must be rejected")
        try session.edit(key: custom, expected: session.snapshot().identity, mutation: .setDisplayName("Keyword"))
        try session.delete(key: custom, expected: session.snapshot().identity)
        let saved = try XCTUnwrap(configuration.codeStyleSheet())
        XCTAssertEqual(saved.withUnsafeBytes { viem_code_replace_style_json($0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count)) }, UInt32(VIEM_STATUS_OK))
        let loaded = try session.snapshot()
        XCTAssertEqual(loaded.definition(for: keyword.key)?.name, "KEYword")
        XCTAssertNil(loaded.definition(for: custom))
        // A live Code buffer may regenerate the name implicitly; the saved file
        // must not resurrect the built-in or the deleted custom definition.
        XCTAssertFalse(loaded.definitions.contains { $0.name == "Keyword" && !$0.flags.contains(.implicit) })
    }

    func testPersistenceFailureRestoresPublishedDefinitionAndDoesNotAddUndo() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let before = try session.snapshot()
        try FileManager.default.removeItem(at: configuration.directory)
        try Data("not a directory".utf8).write(to: configuration.directory)
        XCTAssertThrowsError(try session.edit(key: .baseParagraph, expected: before.identity, mutation: .setDeclaration(.characterSize, .float(27))))
        let after = try session.snapshot()
        XCTAssertEqual(after.definition(for: .baseParagraph)?.properties[.characterSize]?.declared,
                       before.definition(for: .baseParagraph)?.properties[.characterSize]?.declared)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertNotNil(session.lastError)
    }

    func testGlobalChangesReachTwoCodeBuffersWithoutDirtyingOrEnteringDocumentHistory() throws {
        let configuration = configuration()
        let first = EVCoreDocumentBackend(configuration: configuration)
        let second = EVCoreDocumentBackend(configuration: configuration)
        for backend in [first, second] { try backend.read(source: Data("const x = 1;".utf8), typeName: EVDocument.codeType) }
        let surfaces = try [first, second].map { backend in
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            return surface
        }
        let before = try [first, second].map { try $0.recoverySnapshot() }
        let session = try EVCodeStyleSession(configuration: configuration)
        let sheet = try session.snapshot()
        try session.edit(key: .baseParagraph, expected: sheet.identity, mutation: .setDeclaration(.characterSize, .float(19)))
        for (index, backend) in [first, second].enumerated() {
            backend.pollSyntax()
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(19))
            XCTAssertEqual(try backend.recoverySnapshot(), before[index])
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertFalse(surfaces[index].canUndo)
        }
    }

    func testMalformedSavedSheetRemainsReviewableAndExplicitRestoreRepairsIt() throws {
        let configuration = configuration()
        try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
        let file = try XCTUnwrap(configuration.selectedThemeURL)
        let invalid = Data("invalid json".utf8)
        try invalid.write(to: file)
        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertThrowsError(try configuration.reloadCurrentTheme())
        XCTAssertFalse(try session.snapshot().definitions.isEmpty)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
        try session.restoreDefaults()
        XCTAssertNil(session.lastError)
        XCTAssertNotNil(try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    }

    func testExternalStyleReloadUpdatesAllBuffersAndKeepsLastValidStylesOnMalformedOrMissingFile() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let backends = [EVCoreDocumentBackend(configuration: configuration), EVCoreDocumentBackend(configuration: configuration)]
        for backend in backends { try backend.read(source: Data("const x = 1;".utf8), typeName: EVDocument.codeType) }
        try session.edit(key: .baseParagraph, expected: session.snapshot().identity, mutation: .setDeclaration(.characterSize, .float(18)))
        let initial = try session.snapshot()
        let before = try backends.map { try $0.recoverySnapshot() }
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: EVCodeStyleSession.exportGlobalJSON()) as? [String: Any])
        var blocks = try XCTUnwrap(object["block_styles"] as? [[String: Any]])
        let index = try XCTUnwrap(blocks.firstIndex { $0["id"] as? String == "Paragraph" })
        var character = try XCTUnwrap(blocks[index]["character"] as? [String: Any])
        character["size"] = 23
        blocks[index]["character"] = character
        object["block_styles"] = blocks
        let external = try themeData(code: JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]), configuration: configuration)
        let file = try XCTUnwrap(configuration.selectedThemeURL)
        try external.write(to: file, options: .atomic)
        XCTAssertThrowsError(try session.edit(key: .baseParagraph, expected: initial.identity, mutation: .setDeclaration(.characterSize, .float(25))))
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(23))
        XCTAssertGreaterThan(try session.snapshot().identity.styleSheetRevision, initial.identity.styleSheetRevision)
        XCTAssertFalse(session.undoManager.canUndo, "A stale settings undo must not overwrite an external replacement")
        XCTAssertEqual(try Data(contentsOf: file), external, "Reloading does not rewrite external JSON")
        for (index, backend) in backends.enumerated() {
            backend.pollSyntax()
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(23))
            XCTAssertEqual(try backend.recoverySnapshot(), before[index])
        }
        let revision = try session.snapshot().identity.styleSheetRevision
        let malformed = Data("invalid JSON".utf8)
        try malformed.write(to: file, options: .atomic)
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertEqual(try Data(contentsOf: file), malformed)
        XCTAssertTrue(EVCodePreferences.shared.loadDiagnostics.contains { $0.contains("Unable to reload") })
        try FileManager.default.removeItem(at: file)
        await checkExternalChanges()
        XCTAssertNil(configuration.currentThemeName)
        try external.write(to: file, options: .atomic)
        try configuration.selectTheme(named: file.deletingPathExtension().lastPathComponent)
        XCTAssertFalse(EVCodePreferences.shared.loadDiagnostics.contains { $0.contains(file.lastPathComponent) })
    }

    func testLocalStyleWritesAreNotReimportedAndKeepSettingsUndo() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        try session.edit(key: .baseParagraph, expected: session.snapshot().identity, mutation: .setDeclaration(.characterSize, .float(18)))
        let revision = try session.snapshot().identity.styleSheetRevision
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertTrue(session.undoManager.canUndo)
    }

    func testManualReloadRepublishesToEveryBufferEvenWhenTheFileLooksUnchanged() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let backends = [EVCoreDocumentBackend(configuration: configuration),
                        EVCoreDocumentBackend(configuration: configuration)]
        for backend in backends { try backend.read(source: Data("const x = 1;".utf8), typeName: EVDocument.codeType) }
        try session.edit(key: .baseParagraph, expected: session.snapshot().identity,
                         mutation: .setDeclaration(.characterSize, .float(18)))
        let revision = try session.snapshot().identity.styleSheetRevision

        // A local write leaves the stamp current, so the opportunistic check
        // stays a no-op where an explicit reload still republishes.
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        let failure = await reloadStyleSheet()
        XCTAssertNil(failure)
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?
            .properties[.characterSize]?.declared, .float(18))
        XCTAssertTrue(session.undoManager.canUndo,
                      "Reloading identical content is not an external replacement")

        for backend in backends {
            backend.pollSyntax()
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?
                .properties[.characterSize]?.declared, .float(18),
                "Every open buffer follows the republished sheet, not only the focused one")
        }
    }

    func testManualReloadOfAnExternallyRemovedThemeUsesDefault() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let file = try XCTUnwrap(configuration.selectedThemeURL)
        try FileManager.default.removeItem(at: file)
        let failure = await reloadStyleSheet()
        XCTAssertNil(failure)
        XCTAssertNil(configuration.currentThemeName)
        XCTAssertFalse(try session.snapshot().definitions.isEmpty)
    }

    private func themeData(code: Data, configuration: EVConfigurationStore) throws -> Data {
        let file = try XCTUnwrap(configuration.selectedThemeURL)
        var theme = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
        var styles = try XCTUnwrap(theme["styles"] as? [String: Any])
        styles["code"] = try JSONSerialization.jsonObject(with: code)
        theme["styles"] = styles
        return try JSONSerialization.data(withJSONObject: theme, options: [.sortedKeys])
    }

    private func checkExternalChanges() async {
        await withCheckedContinuation { continuation in
            EVCodeStyleSession.checkExternalStyleChanges { continuation.resume() }
        }
    }

    private func reloadStyleSheet() async -> String? {
        await withCheckedContinuation { continuation in
            EVCodeStyleSession.reloadStyleSheet { continuation.resume(returning: $0) }
        }
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }
}
