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

    func testNativeCodeMenusMarkTheActiveSyntaxAndBaseStylesAndValidationUpdatesMovement() async throws {
        let surface = try surface()
        try await waitForRustHighlighting(surface)
        let before = try surface.backend.recoverySnapshot()
        let owner = EVApplicationDelegate(configuration: surface.backend.configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let character = try XCTUnwrap(main.item(withTitle: "Character")?.submenu)
        let paragraph = try XCTUnwrap(main.item(withTitle: "Paragraph")?.submenu)
        builder.menuNeedsUpdate(character)
        builder.menuNeedsUpdate(paragraph)
        let keyword = try XCTUnwrap(character.item(withTitle: "Keyword"))
        let baseCharacter = try XCTUnwrap(character.item(withTitle: "Default Paragraph"))
        let baseParagraph = try XCTUnwrap(paragraph.item(withTitle: "Base Paragraph"))
        let characterFooter = try XCTUnwrap(character.item(withTitle: "Edit Styles…"))
        let paragraphFooter = try XCTUnwrap(paragraph.item(withTitle: "Edit Styles…"))
        XCTAssertTrue(character.showsStateColumn)
        XCTAssertTrue(paragraph.showsStateColumn)
        XCTAssertEqual(keyword.state, .on)
        XCTAssertEqual(baseCharacter.state, .off)
        XCTAssertEqual(baseParagraph.state, .on)

        for menu in [character, paragraph] {
            for item in menu.items where item.representedObject is EVStyleMenuAction {
                XCTAssertTrue(surface.editorView.validateMenuItem(item), item.title)
            }
        }
        XCTAssertEqual(keyword.state, .on, "Native validation must preserve the syntax checkmark")
        XCTAssertEqual(baseParagraph.state, .on)
        XCTAssertEqual(characterFooter.state, .off)
        XCTAssertEqual(paragraphFooter.state, .off)

        // Keep the existing NSMenuItem objects as AppKit does while tracking;
        // validation must refresh selection state without rebuilding the menu.
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 0))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 2)
        XCTAssertTrue(surface.editorView.validateMenuItem(keyword))
        XCTAssertTrue(surface.editorView.validateMenuItem(baseCharacter))
        XCTAssertEqual(keyword.state, .off)
        XCTAssertEqual(baseCharacter.state, .on, "The space after fn has the default character style")
        characterFooter.state = .on
        paragraphFooter.state = .on
        XCTAssertTrue(surface.editorView.validateMenuItem(characterFooter))
        XCTAssertTrue(surface.editorView.validateMenuItem(paragraphFooter))
        XCTAssertEqual(characterFooter.state, .off, "The generic editor command is never a selected style")
        XCTAssertEqual(paragraphFooter.state, .off)

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 0))
        XCTAssertTrue(surface.editorView.validateMenuItem(keyword))
        XCTAssertTrue(surface.editorView.validateMenuItem(baseCharacter))
        XCTAssertEqual(keyword.state, .on)
        XCTAssertEqual(baseCharacter.state, .off)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
    }

    func testCodeMenuChecksOneUniformSyntaxSelectionAndClearsCharacterChecksForMixedText() async throws {
        let surface = try surface()
        try await waitForRustHighlighting(surface)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 2))
        var catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertEqual(catalogue.entries.filter { $0.role == .character && $0.presentation.state == .on }
            .map(\.displayName), ["Keyword"])
        XCTAssertEqual(catalogue.entries.first { $0.role == .paragraph && $0.stableID == "Paragraph" }?
            .presentation.state, .on)

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 3))
        catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertTrue(try XCTUnwrap(surface.session).selectedNamedStyles().characterMixed)
        XCTAssertTrue(catalogue.entries.filter { $0.role == .character }
            .allSatisfy { $0.presentation.state == .off }, "A mixed syntax/default selection has no unique character style")
        XCTAssertEqual(catalogue.entries.first { $0.role == .paragraph && $0.stableID == "Paragraph" }?
            .presentation.state, .on)
        XCTAssertFalse(surface.canUndo)
    }

    func testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget() throws {
        let surface = try surface()
        let before = try surface.backend.recoverySnapshot()
        let code = try EVCodeStyleSession(configuration: surface.backend.configuration)
        let initial = try code.snapshot()
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let characters = catalogue.entries.filter { $0.role == .character }
        XCTAssertEqual(Set(characters.map(\.stableID)),
                       Set(initial.definitions.filter { $0.kind == .character && !$0.flags.contains(.internalSyntax) }.map { $0.key.id.rawValue } + [""]))
        XCTAssertTrue(characters.allSatisfy { $0.actionKind == .edit && $0.presentation.isEnabled })
        let keyword = try XCTUnwrap(characters.first { $0.displayName == "Keyword" })
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
        let keyword = try XCTUnwrap(initial.definitions.first { $0.name == "Keyword" })
        try code.delete(key: keyword.key, expected: initial.identity)
        let before = try surface.backend.recoverySnapshot()
        let missingRevision = try code.snapshot().identity.styleSheetRevision
        for _ in 0..<200 {
            surface.backend.pollSyntax()
            if try surface.backend.syntaxStyleNames().contains("Keyword") { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(try surface.backend.syntaxStyleNames().contains("Keyword"))
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.syntaxName == "Keyword" })
        XCTAssertEqual(entry.actionKind, .defineSyntax)
        XCTAssertEqual(entry.displayName, "Define Keyword…")
        XCTAssertEqual(entry.stableID, "", "An unresolved reference has no invented definition ID")
        XCTAssertEqual(entry.presentation.state, .off)
        XCTAssertEqual(catalogue.entries.first { $0.role == .character && $0.stableID == "" && $0.isBase }?
            .presentation.state, .on, "An unresolved syntax style renders with Default Paragraph")
        XCTAssertEqual(try code.snapshot().identity.styleSheetRevision, missingRevision)
        XCTAssertFalse(try code.snapshot().definitions.contains { $0.name == "Keyword" })
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        let item = menuItem(entry, catalogue: catalogue)
        XCTAssertTrue(surface.editorView.validateMenuItem(item))
        surface.editorView.performEditorStyleMenuAction(item)
        let defined = try XCTUnwrap(code.snapshot().definitions.first { $0.name == "Keyword" })
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, defined.key)
        XCTAssertNotEqual(defined.key, keyword.key)
        XCTAssertFalse(defined.properties.values.contains { $0.declared != nil },
                       "Explicit creation inherits the default appearance until properties are changed")
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.editorView.validateMenuItem(item), "An already defined name cannot be created twice")
        let fresh = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertEqual(fresh.entries.first { $0.displayName == "Keyword" }?.actionKind, .edit)
        XCTAssertFalse(fresh.entries.contains { $0.syntaxName == "Keyword" })
    }

    func testCodeEditStylesCommandsTargetBasesAndAssignmentsStayDisabled() throws {
        let surface = try surface()
        let before = try surface.backend.recoverySnapshot()
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        for (command, key) in [(EVMenuCommand.editCharacterStyles, EVStyleKey.baseParagraph),
                               (.editParagraphStyles, .baseParagraph), (.editStyles, .baseParagraph)] {
            XCTAssertTrue(surface.presentation(for: command).isEnabled)
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, key)
            XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        }
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.displayName == "Keyword" })
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

    private func waitForRustHighlighting(_ surface: EVEditorSurfaceController) async throws {
        for _ in 0..<200 {
            surface.backend.pollSyntax()
            surface.refreshPresentation()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= 2
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rust fn did not receive syntax highlighting: \(surface.statusBarState.message)")
    }
}
