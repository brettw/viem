import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVCodeStyleMenuTests: XCTestCase {
    private func surface() throws -> EVEditorSurfaceController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-style-menu-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data("fn main() { let count = 42; }\n".utf8),
                         typeName: EVDocument.codeType, filename: "main.rs", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
    }

    private func menuItem(_ entry: EVStyleMenuEntry, catalogue: EVStyleMenuCatalogue) -> NSMenuItem {
        let item = NSMenuItem(title: entry.displayName,
                              action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)),
                              keyEquivalent: "")
        item.representedObject = EVStyleMenuAction(
            kind: entry.actionKind, role: entry.role, stableID: entry.stableID,
            documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
            styleSheetRevision: catalogue.styleSheetRevision, syntaxName: entry.syntaxName)
        return item
    }

    func testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget() throws {
        let surface = try surface()
        let before = try surface.backend.recoverySnapshot()
        let code = try EVCodeStyleSession(configuration: surface.backend.configuration)
        let initial = try code.snapshot()
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let characters = catalogue.entries.filter { $0.role == .character }
        XCTAssertEqual(Set(characters.map(\.stableID)),
                       Set(initial.definitions.filter { $0.kind == .character }.map { $0.key.id.rawValue }))
        XCTAssertTrue(characters.allSatisfy { $0.actionKind == .edit && $0.presentation.isEnabled })
        let keyword = try XCTUnwrap(characters.first { $0.displayName == "@keyword" })
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        let item = menuItem(keyword, catalogue: catalogue)
        XCTAssertTrue(surface.editorView.validateMenuItem(item))
        surface.editorView.performEditorStyleMenuAction(item)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey,
                       EVStyleKey(namespace: .character, id: EVStyleID(rawValue: keyword.stableID)))
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertFalse(try XCTUnwrap(coordinator.inspection).hasDocument)
        let editor = try XCTUnwrap(coordinator.styleWindow?.contentViewController as? EVStyleEditorViewController)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(27)), editor.inspection.diagnostic)
        XCTAssertEqual(try code.snapshot().definition(namespace: .character, id: EVStyleID(rawValue: keyword.stableID))?
            .properties[.characterSize]?.declared, .float(27))
        XCTAssertNotNil(try surface.backend.configuration.codeStyleSheet())
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.backend.persistenceState.isDirty)
        surface.editorView.performEditorStyleMenuAction(item)
        XCTAssertEqual(coordinator.inspection?.selectedStyleID?.rawValue, keyword.stableID,
                       "A harmless stylesheet revision change does not lose the stable edit target")
    }

    func testUndefinedSyntaxNameAppearsWithoutCreatingAStyleAndDefineIsExplicit() async throws {
        let surface = try surface()
        let code = try EVCodeStyleSession(configuration: surface.backend.configuration)
        let initial = try code.snapshot()
        let keyword = try XCTUnwrap(initial.definitions.first { $0.name == "@keyword" })
        try code.delete(key: keyword.key, expected: initial.identity)
        let before = try surface.backend.recoverySnapshot()
        let missingRevision = try code.snapshot().identity.styleSheetRevision
        for _ in 0..<200 {
            surface.backend.pollSyntax()
            if try surface.backend.syntaxStyleNames().contains("@keyword") { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(try surface.backend.syntaxStyleNames().contains("@keyword"))
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.syntaxName == "@keyword" })
        XCTAssertEqual(entry.actionKind, .defineSyntax)
        XCTAssertEqual(entry.displayName, "Define @keyword…")
        XCTAssertEqual(entry.stableID, "", "An unresolved reference has no invented definition ID")
        XCTAssertEqual(try code.snapshot().identity.styleSheetRevision, missingRevision)
        XCTAssertFalse(try code.snapshot().definitions.contains { $0.name == "@keyword" })
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        let item = menuItem(entry, catalogue: catalogue)
        XCTAssertTrue(surface.editorView.validateMenuItem(item))
        surface.editorView.performEditorStyleMenuAction(item)
        let defined = try XCTUnwrap(code.snapshot().definitions.first { $0.name == "@keyword" })
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, defined.key)
        XCTAssertNotEqual(defined.key, keyword.key)
        XCTAssertFalse(defined.properties.values.contains { $0.declared != nil },
                       "Explicit creation inherits the default appearance until properties are changed")
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.editorView.validateMenuItem(item), "An already defined name cannot be created twice")
        let fresh = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertEqual(fresh.entries.first { $0.displayName == "@keyword" }?.actionKind, .edit)
        XCTAssertFalse(fresh.entries.contains { $0.syntaxName == "@keyword" })
    }

    func testCodeEditStylesCommandsTargetBasesAndAssignmentsStayDisabled() throws {
        let surface = try surface()
        let before = try surface.backend.recoverySnapshot()
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        for (command, key) in [(EVMenuCommand.editCharacterStyles, EVStyleKey.baseCharacter),
                               (.editParagraphStyles, .baseParagraph), (.editDocumentStyles, .baseDocument)] {
            XCTAssertTrue(surface.presentation(for: command).isEnabled)
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, key)
            XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        }
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.displayName == "@keyword" })
        coordinator.close()
        let forged = NSMenuItem(title: "Assign", action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)), keyEquivalent: "")
        forged.representedObject = EVStyleMenuAction(kind: .assign, role: .character, stableID: entry.stableID,
            documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
            styleSheetRevision: catalogue.styleSheetRevision)
        XCTAssertFalse(surface.editorView.validateMenuItem(forged))
        surface.editorView.performEditorStyleMenuAction(forged)
        XCTAssertNil(coordinator.styleWindow)
        XCTAssertFalse(surface.presentation(for: .saveDefaultStyle).isEnabled)
        XCTAssertFalse(surface.presentation(for: .bold).isEnabled)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
    }
}
