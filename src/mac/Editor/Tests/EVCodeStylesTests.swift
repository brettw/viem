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

    func testGlobalEditorHasIndependentTargetUndoAndPersistence() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let editor = EVStyleEditorViewController()
        editor.retarget(codeSession: session)
        XCTAssertNil(editor.inspection.targetDocumentIdentity)
        XCTAssertEqual(editor.inspection.targetCoreDocumentID, 0)
        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseDocument)
        XCTAssertFalse(editor.inspection.hasDocument)
        XCTAssertTrue(editor.inspection.mutationsEnabled)
        let initial = try session.snapshot()
        let originalSize = initial.definition(for: .baseDocument)?.properties[.characterSize]?.declared
        editor.beginContinuousStyleEditForTesting()
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(18)), editor.inspection.diagnostic)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(20)), editor.inspection.diagnostic)
        editor.endContinuousStyleEditForTesting()
        XCTAssertTrue(session.undoManager.canUndo)
        XCTAssertEqual(try session.snapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(20))
        let committedRevision = try session.snapshot().identity.styleSheetRevision
        session.undoManager.undo()
        XCTAssertEqual(try session.snapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, originalSize)
        XCTAssertFalse(session.undoManager.canUndo, "The continuous edit is one settings transaction")
        XCTAssertGreaterThan(try session.snapshot().identity.styleSheetRevision, committedRevision)
        session.undoManager.redo()
        XCTAssertEqual(try session.snapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(20))
        let data = try XCTUnwrap(configuration.codeStyleSheet())
        XCTAssertEqual(viem_code_replace_style_json(nil, 0), UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(data.withUnsafeBytes { viem_code_replace_style_json($0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count)) }, UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(try session.snapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(20))
        XCTAssertNil(try configuration.styleDefaults(named: "text"))
    }

    func testNamesAreExactAndDeletionAndRenamePersistWithoutResurrectingBuiltins() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let editor = EVStyleEditorViewController()
        editor.retarget(codeSession: session)
        let initial = try session.snapshot()
        let keyword = try XCTUnwrap(initial.definitions.first { $0.name == "@keyword" })
        editor.selectStyle(keyword.key)
        XCTAssertTrue(editor.renameForTesting("@Keyword"), editor.inspection.diagnostic)
        XCTAssertTrue(editor.createStyle(kind: .character), editor.inspection.diagnostic)
        let custom = try XCTUnwrap(editor.inspection.selectedStyleKey)
        XCTAssertFalse(editor.renameForTesting("@Keyword"), "Exact duplicate names must be rejected")
        XCTAssertTrue(editor.renameForTesting("@keyword"), editor.inspection.diagnostic)
        XCTAssertTrue(editor.deleteSelectedStyle(), editor.inspection.diagnostic)
        let saved = try XCTUnwrap(configuration.codeStyleSheet())
        XCTAssertEqual(saved.withUnsafeBytes { viem_code_replace_style_json($0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count)) }, UInt32(VIEM_STATUS_OK))
        let loaded = try session.snapshot()
        XCTAssertEqual(loaded.definition(for: keyword.key)?.name, "@Keyword")
        XCTAssertNil(loaded.definition(for: custom))
        XCTAssertFalse(loaded.definitions.contains { $0.name == "@keyword" })
        XCTAssertFalse(editor.createStyle(kind: .paragraph))
    }

    func testPersistenceFailureRestoresPublishedDefinitionAndDoesNotAddUndo() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let before = try session.snapshot()
        try Data("not a directory".utf8).write(to: configuration.directory)
        XCTAssertThrowsError(try session.edit(key: .baseDocument, expected: before.identity, mutation: .setDeclaration(.characterSize, .float(27))))
        let after = try session.snapshot()
        XCTAssertEqual(after.definition(for: .baseDocument)?.properties[.characterSize]?.declared,
                       before.definition(for: .baseDocument)?.properties[.characterSize]?.declared)
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
        try session.edit(key: .baseDocument, expected: sheet.identity, mutation: .setDeclaration(.characterSize, .float(19)))
        for (index, backend) in [first, second].enumerated() {
            backend.pollSyntax()
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(19))
            XCTAssertEqual(try backend.recoverySnapshot(), before[index])
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertFalse(surfaces[index].canUndo)
        }
    }

    func testMalformedSavedSheetRemainsReviewableAndExplicitRestoreRepairsIt() throws {
        let configuration = configuration()
        try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
        let file = configuration.directory.appendingPathComponent("code_style.json")
        let invalid = Data("invalid json".utf8)
        try invalid.write(to: file)
        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertNotNil(session.lastError)
        XCTAssertFalse(try session.snapshot().definitions.isEmpty)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
        try session.restoreDefaults()
        XCTAssertNil(session.lastError)
        XCTAssertNotNil(try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    }

    func testGlobalMetricsChangesKeepViewportTextAnchoredAcrossTwoBuffers() throws {
        let configuration = configuration()
        let source = (0..<2_000).map { "Line \($0): a short code statement" }.joined(separator: "\n")
        let backends = [EVCoreDocumentBackend(configuration: configuration), EVCoreDocumentBackend(configuration: configuration)]
        let surfaces = try backends.enumerated().map { index, backend in
            try backend.read(source: Data(source.utf8), typeName: EVDocument.codeType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            surface.view.frame = NSRect(x: 0, y: 0, width: 560, height: 180)
            surface.viewDidLayout()
            let session = try XCTUnwrap(surface.session)
            surface.performInput { _ = try session.sendText("\(301 + index * 500)Gzt") }
            if index == 1 {
                // An offscreen caret must not pull a view back during global
                // style changes. This view retains its first visible text.
                surface.requestVerticalViewport(top: CGFloat(surface.viewportState.top + 500))
            }
            return surface
        }
        func topRow(_ surface: EVEditorSurfaceController) throws -> ViemVisualRowV1 {
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            return try XCTUnwrap(snapshot.rows.first { $0.y + $0.line_advance > surface.viewportState.top + 0.01 })
        }
        let before = try surfaces.map { try topRow($0) }
        let beforeOffsets = zip(before, surfaces).map { $0.y - $1.viewportState.top }
        XCTAssertLessThan(surfaces[1].viewPresentation.cursor_utf8_offset, before[1].text_start)
        func caretRow(_ surface: EVEditorSurfaceController) throws -> ViemVisualRowV1 {
            let layout = try XCTUnwrap(surface.layoutSnapshot)
            let presentation = surface.viewPresentation
            let caret = try XCTUnwrap(surface.session).caretGeometry(offset: presentation.cursor_utf8_offset,
                affinity: presentation.cursor_affinity, in: layout.info)
            return try XCTUnwrap(layout.rows.first { $0.row_index == caret.row_index })
        }
        var expectedCaretBaseline = try caretRow(surfaces[0]).baseline - surfaces[0].viewportState.top
        let original = try backends.map { try $0.recoverySnapshot() }
        let session = try EVCodeStyleSession(configuration: configuration)
        for (key, mutation) in [(EVStyleKey.baseDocument, EVStyleMutation.setDeclaration(.characterSize, .float(23))),
                         (.baseParagraph, .setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: 38))))] {
            try session.edit(key: key, expected: session.snapshot().identity, mutation: mutation)
            for (index, backend) in backends.enumerated() {
                backend.pollSyntax()
                let row = try topRow(surfaces[index])
                let layout = try XCTUnwrap(surfaces[index].layoutSnapshot)
                if index == 0 {
                    // The visible caret wins over the first-row anchor. At
                    // zt's top edge, growing text may shift its baseline only
                    // as far as required to show the complete row and ink.
                    let caret = try caretRow(surfaces[index])
                    let ink = layout.clusters.filter { $0.row_index == caret.row_index }.map(\.ink_bounds)
                    let top = min(caret.y, ink.map(\.y).min() ?? caret.y)
                    let naturalBottom = caret.y + caret.ascent + caret.descent + caret.leading
                    let bottom = max(naturalBottom, ink.map { $0.y + $0.height }.max() ?? naturalBottom)
                    expectedCaretBaseline = min(max(expectedCaretBaseline, caret.baseline - top),
                        layout.info.viewport_height - (bottom - caret.baseline))
                    XCTAssertEqual(caret.baseline - surfaces[index].viewportState.top,
                                   expectedCaretBaseline, accuracy: 0.5)
                    XCTAssertGreaterThanOrEqual(top - surfaces[index].viewportState.top, -0.5)
                    XCTAssertLessThanOrEqual(bottom - surfaces[index].viewportState.top,
                                            layout.info.viewport_height + 0.5)
                } else {
                    XCTAssertEqual(row.text_start, before[index].text_start)
                    XCTAssertEqual(row.y - surfaces[index].viewportState.top, beforeOffsets[index], accuracy: 0.5)
                }
                XCTAssertLessThan(layout.rows.count, 100)
                XCTAssertLessThan(layout.clusters.count, 6_000)
                XCTAssertEqual(try backend.recoverySnapshot(), original[index])
                XCTAssertFalse(surfaces[index].canUndo)
            }
        }
        for surface in surfaces { XCTAssertEqual(try topRow(surface).line_advance, 38, accuracy: 0.5) }
    }

    func testExternalStyleReloadUpdatesAllBuffersAndKeepsLastValidStylesOnMalformedOrMissingFile() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let backends = [EVCoreDocumentBackend(configuration: configuration), EVCoreDocumentBackend(configuration: configuration)]
        for backend in backends { try backend.read(source: Data("const x = 1;".utf8), typeName: EVDocument.codeType) }
        try session.edit(key: .baseDocument, expected: session.snapshot().identity, mutation: .setDeclaration(.characterSize, .float(18)))
        let initial = try session.snapshot()
        let before = try backends.map { try $0.recoverySnapshot() }
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: EVCodeStyleSession.exportGlobalJSON()) as? [String: Any])
        var blocks = try XCTUnwrap(object["block_styles"] as? [[String: Any]])
        let index = try XCTUnwrap(blocks.firstIndex { $0["id"] as? String == "Document" })
        var character = try XCTUnwrap(blocks[index]["character"] as? [String: Any])
        character["size"] = 23
        blocks[index]["character"] = character
        object["block_styles"] = blocks
        let external = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        let file = configuration.directory.appendingPathComponent("code_style.json")
        try external.write(to: file, options: .atomic)
        XCTAssertThrowsError(try session.edit(key: .baseDocument, expected: initial.identity, mutation: .setDeclaration(.characterSize, .float(25))))
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(23))
        XCTAssertGreaterThan(try session.snapshot().identity.styleSheetRevision, initial.identity.styleSheetRevision)
        XCTAssertFalse(session.undoManager.canUndo, "A stale settings undo must not overwrite an external replacement")
        XCTAssertEqual(try Data(contentsOf: file), external, "Reloading does not rewrite external JSON")
        for (index, backend) in backends.enumerated() {
            backend.pollSyntax()
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseDocument)?.properties[.characterSize]?.declared, .float(23))
            XCTAssertEqual(try backend.recoverySnapshot(), before[index])
        }
        let revision = try session.snapshot().identity.styleSheetRevision
        let malformed = Data("invalid JSON".utf8)
        try malformed.write(to: file, options: .atomic)
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertEqual(try Data(contentsOf: file), malformed)
        XCTAssertTrue(EVCodePreferences.shared.loadDiagnostics.contains { $0.contains("Unable to reload code_style.json") })
        try FileManager.default.removeItem(at: file)
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertTrue(EVCodePreferences.shared.loadDiagnostics.contains { $0.contains("was removed") })
        try external.write(to: file, options: .atomic)
        await checkExternalChanges()
        XCTAssertFalse(EVCodePreferences.shared.loadDiagnostics.contains { $0.contains("code_style.json") })
    }

    func testLocalStyleWritesAreNotReimportedAndKeepSettingsUndo() async throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        try session.edit(key: .baseDocument, expected: session.snapshot().identity, mutation: .setDeclaration(.characterSize, .float(18)))
        let revision = try session.snapshot().identity.styleSheetRevision
        await checkExternalChanges()
        XCTAssertEqual(try session.snapshot().identity.styleSheetRevision, revision)
        XCTAssertTrue(session.undoManager.canUndo)
    }

    private func checkExternalChanges() async {
        await withCheckedContinuation { continuation in
            EVCodeStyleSession.checkExternalStyleChanges { continuation.resume() }
        }
    }
}
