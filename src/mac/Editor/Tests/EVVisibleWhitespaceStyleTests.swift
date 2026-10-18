import AppKit
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVVisibleWhitespaceStyleTests: XCTestCase {
    private func fixture() -> (URL, EVConfigurationStore) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-marker-style-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (directory, EVConfigurationStore(directory: directory))
    }

    func testAllCharacterPropertiesPersistAndClearAtomically() throws {
        let (directory, configuration) = fixture()
        let session = EVVisibleWhitespaceStyleSession(configuration: configuration)
        let preferences = EVEditingPreferences(configuration: configuration)
        let mutations: [EVStyleMutation] = [
            .setDeclaration(.characterFontFamilies, .stringList(["Helvetica", "Times New Roman"])),
            .setDeclaration(.characterSize, .float(18)),
            .setDeclaration(.characterWeight, .unsigned(450)),
            .setDeclaration(.characterBold, .boolean(true)),
            .setDeclaration(.characterSlant, .fontSlant(2)),
            .setDeclaration(.characterForeground, .color(EVStyleColor(red: 0.2, green: 0.3, blue: 0.4, alpha: 1))),
            .setDeclaration(.characterBackground, .color(EVStyleColor(red: 1, green: 1, blue: 0, alpha: 0.5))),
            .setDeclaration(.characterUnderline, .boolean(true)),
            .setDeclaration(.characterStrikethrough, .boolean(false)),
            .setDeclaration(.characterLanguage, .string("fr")),
            .setDeclaration(.characterDirection, .writingDirection(2)),
            .setDeclaration(.characterOpenTypeFeatures, .openTypeFeatures([EVOpenTypeFeature(tag: "liga", setting: 0)])),
            .setDeclaration(.characterLetterSpacing, .float(0.25)),
        ]
        XCTAssertTrue(session.apply(mutations))
        let style = session.style
        XCTAssertEqual(style.size, 18)
        XCTAssertEqual(style.fontFamilies, ["Helvetica", "Times New Roman"])
        XCTAssertEqual(style.weight, 450)
        XCTAssertEqual(style.slant, .oblique)
        XCTAssertEqual(style.direction, .rightToLeft)
        XCTAssertEqual(style.language, "fr")
        XCTAssertEqual(style.openTypeFeatures, ["liga": 0])
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style, style)
        XCTAssertEqual(preferences.whitespacePresentation.visibleWhitespace.style, style)
        XCTAssertEqual(EVConfigurationStore(directory: directory).whitespacePresentation.visibleWhitespace.style, style)
        XCTAssertTrue(session.apply(EVStyleProperty.characterProperties.map(EVStyleMutation.clearDeclaration)))
        XCTAssertEqual(session.style, EVVisibleWhitespaceStyle())
        XCTAssertEqual(EVConfigurationStore(directory: directory).whitespacePresentation.visibleWhitespace.style, EVVisibleWhitespaceStyle())
        session.undoManager.undo()
        XCTAssertEqual(session.style, style)
    }

    func testLiveGestureIsOneUndoAndRedoPreservesUnrelatedSettings() throws {
        let (_, configuration) = fixture()
        let session = EVVisibleWhitespaceStyleSession(configuration: configuration)
        session.beginGroup()
        XCTAssertTrue(session.apply([.setDeclaration(.characterSize, .float(16))]))
        XCTAssertTrue(session.apply([.setDeclaration(.characterSize, .float(18))]))
        XCTAssertFalse(session.undoManager.canUndo)
        session.endGroup()
        XCTAssertTrue(session.undoManager.canUndo)
        var options = configuration.indentation
        options.tabstop = 8
        try configuration.setIndentation(options)
        var presentation = configuration.whitespacePresentation
        presentation.visibleWhitespace.enabled = false
        try configuration.setWhitespacePresentation(presentation)
        session.undoManager.undo()
        XCTAssertNil(session.style.size)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertTrue(session.undoManager.canRedo)
        XCTAssertEqual(configuration.indentation.tabstop, 8)
        XCTAssertFalse(configuration.whitespacePresentation.visibleWhitespace.enabled)
        session.undoManager.redo()
        XCTAssertEqual(session.style.size, 18)
        XCTAssertEqual(configuration.indentation.tabstop, 8)
        XCTAssertTrue(session.restoreDefaults())
        XCTAssertEqual(session.style, .defaultStyle)
        session.undoManager.undo()
        XCTAssertEqual(session.style.size, 18)
    }

    func testInvalidMutationsAndWriteFailureLeaveStyleAndHistoryUnchanged() throws {
        let (directory, configuration) = fixture()
        let session = EVVisibleWhitespaceStyleSession(configuration: configuration)
        XCTAssertFalse(session.apply([.setDeclaration(.characterSize, .float(0))]))
        XCTAssertFalse(session.apply([.setDeclaration(.characterWeight, .unsigned(1001))]))
        XCTAssertFalse(session.apply([.setDeclaration(.characterLanguage, .string(""))]))
        XCTAssertFalse(session.apply([.setDeclaration(.paragraphFirstLineIndent, .float(4))]))
        XCTAssertFalse(session.apply([.setDisplayName("Other")]))
        XCTAssertEqual(session.style, .defaultStyle)
        XCTAssertFalse(session.undoManager.canUndo)
        try FileManager.default.removeItem(at: directory)
        try Data("obstruction".utf8).write(to: directory)
        XCTAssertFalse(session.apply([.setDeclaration(.characterBold, .boolean(true))]))
        XCTAssertEqual(session.style, .defaultStyle)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertEqual(try Data(contentsOf: directory), Data("obstruction".utf8))
    }

    func testExternalStyleCommitRefreshesControlsAndDiscardsStaleUndo() throws {
        let (directory, configuration) = fixture()
        let session = EVVisibleWhitespaceStyleSession(configuration: configuration)
        XCTAssertTrue(session.apply([.setDeclaration(.characterSize, .float(16))]))
        XCTAssertTrue(session.undoManager.canUndo)
        let other = EVConfigurationStore(directory: directory)
        var settings = other.whitespacePresentation
        settings.visibleWhitespace.style.size = 22
        try other.setWhitespacePresentation(settings)
        XCTAssertEqual(session.style.size, 22)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(session.undoManager.canRedo)
    }

    func testSettingsButtonReusesOrdinaryStyleEditorAndKeepsAnExplicitWhitespaceTarget() throws {
        let (directory, configuration) = fixture()
        var presentation = configuration.whitespacePresentation
        presentation.visibleWhitespace.style.size = 14
        try configuration.setWhitespacePresentation(presentation)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Heading\n\nBody text".utf8)
        try backend.read(source: source, typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 240)
        surface.viewDidLayout()

        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        let previousRoute = EVEditingPreferences.editVisibleWhitespaceStyle
        EVEditorComposition.install()
        defer {
            coordinator.close()
            EVEditingPreferences.editVisibleWhitespaceStyle = previousRoute
        }
        coordinator.show(document: surface, sender: nil)
        let originalWindow = try XCTUnwrap(coordinator.styleWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey,
            EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
        let themeBefore = try configuration.codeStyleSheet()
        let settings = EVSettingsWindowController(store: EVThemeStore(configuration: configuration),
            editingPreferences: EVEditingPreferences(configuration: configuration),
            viewPreferences: EVViewPreferences(configuration: configuration))
        defer { settings.close() }
        settings.showWindow(nil)
        let settingsWindow = try XCTUnwrap(settings.window)
        let sidebar = try XCTUnwrap(descendants(settingsWindow.contentView).compactMap { $0 as? NSTableView }.first)
        sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
        let editStyle = try XCTUnwrap(descendants(settingsWindow.contentView).compactMap { $0 as? NSButton }
            .first { $0.title == "Edit Style…" })
        editStyle.performClick(nil)

        XCTAssertTrue(coordinator.styleWindow === originalWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .visibleWhitespace)
        XCTAssertEqual(originalWindow.title, "Theme styles — Code — \(configuration.currentThemeName ?? "Default")")
        XCTAssertNil(originalWindow.sheetParent)
        let editor = try XCTUnwrap(originalWindow.contentViewController as? EVStyleEditorViewController)
        let controls = descendants(editor.view)
        XCTAssertNotNil(controls.first { $0.accessibilityLabel() == "Font family" })
        XCTAssertNotNil(controls.first { $0.accessibilityLabel() == "Live style preview" })
        let basedOn = try XCTUnwrap(controls.first { $0.accessibilityLabel() == "Based on style" } as? NSPopUpButton)
        XCTAssertFalse(basedOn.isEnabled)
        XCTAssertEqual(basedOn.titleOfSelectedItem, "Default Paragraph")
        let size = try XCTUnwrap(controls.first { $0.accessibilityLabel() == "Size" } as? NSTextField)
        XCTAssertTrue(size.isEnabled)
        originalWindow.makeKeyAndOrderFront(nil)
        XCTAssertTrue(originalWindow.makeFirstResponder(size))
        let fieldEditor = try XCTUnwrap(size.currentEditor() as? NSTextView)
        fieldEditor.selectAll(nil)
        fieldEditor.insertText("20", replacementRange: fieldEditor.selectedRange())
        fieldEditor.insertTab(nil)
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 20)
        XCTAssertEqual(EVConfigurationStore(directory: directory).whitespacePresentation.visibleWhitespace.style.size, 20)
        let undo = try XCTUnwrap(originalWindow.undoManager)
        XCTAssertTrue(undo.canUndo)
        undo.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 14)
        undo.redo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 20)

        editStyle.performClick(nil)
        XCTAssertTrue(coordinator.styleWindow === originalWindow)
        XCTAssertTrue(originalWindow.undoManager === undo)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .visibleWhitespace)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 8, length: 0))
        coordinator.settleSelectionFollowForTesting()
        let otherBackend = EVCoreDocumentBackend(configuration: configuration)
        try otherBackend.read(source: Data("other".utf8), typeName: EVDocument.plainTextType)
        let otherSurface = try XCTUnwrap(otherBackend.makeEditorSurface() as? EVEditorSurfaceController)
        otherSurface.loadViewIfNeeded()
        coordinator.documentDidBecomeActive(otherSurface)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .visibleWhitespace)
        XCTAssertEqual(try configuration.codeStyleSheet(), themeBefore)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        XCTAssertFalse(surface.canUndo, "Settings edits must not enter document undo history")
    }

    func testCodeAndWhitespaceStylesShareChronologicalUndoWithoutChangingPersistenceAuthority() throws {
        let (directory, configuration) = fixture()
        let session = try EVCodeStyleSession(configuration: configuration)
        let original = try session.snapshot()
        let baseSize = original.definition(for: .baseParagraph)?.properties[.characterSize]?.declared
        let themeBefore = try configuration.codeStyleSheet()
        try session.beginGroup()
        try session.edit(key: .visibleWhitespace, expected: session.snapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(16)))
        try session.edit(key: .visibleWhitespace, expected: session.snapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(18)))
        session.endGroup()
        XCTAssertEqual(try configuration.codeStyleSheet(), themeBefore)
        try session.edit(key: .baseParagraph, expected: session.snapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(23)))
        let changedTheme = try configuration.codeStyleSheet()
        try session.edit(key: .visibleWhitespace, expected: session.snapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(22)))
        XCTAssertEqual(try configuration.codeStyleSheet(), changedTheme)

        session.undoManager.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 18)
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(23))
        session.undoManager.undo()
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, baseSize)
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 18)
        session.undoManager.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style, .defaultStyle)
        XCTAssertFalse(session.undoManager.canUndo)
        for _ in 0..<3 { session.undoManager.redo() }
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 22)
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(23))
        XCTAssertEqual(EVConfigurationStore(directory: directory).whitespacePresentation.visibleWhitespace.style.size, 22)
        try session.restoreDefaults(for: .visibleWhitespace)
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style, .defaultStyle)
        XCTAssertEqual(try configuration.codeStyleSheet(), changedTheme)
        session.undoManager.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 22)
    }

    func testWhitespaceSnapshotRejectsStaleCallbacksAfterUndoAndExternalChanges() throws {
        let (directory, configuration) = fixture()
        let session = try EVCodeStyleSession(configuration: configuration)
        let original = try session.snapshot()
        try session.edit(key: .visibleWhitespace, expected: original.identity,
            mutation: .setDeclaration(.characterBold, .boolean(true)))
        session.undoManager.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style, .defaultStyle)
        XCTAssertNotEqual(try session.snapshot().identity, original.identity,
            "Undo restores values without reviving an old settings snapshot")
        XCTAssertThrowsError(try session.edit(key: .visibleWhitespace, expected: original.identity,
            mutation: .setDeclaration(.characterSize, .float(99)))) { error in
            XCTAssertTrue((error as? EVStyleBridgeError)?.isStale == true)
        }
        XCTAssertNil(configuration.whitespacePresentation.visibleWhitespace.style.size)
        XCTAssertTrue(session.undoManager.canRedo)
        let beforeExternal = try session.snapshot()
        let other = EVConfigurationStore(directory: directory)
        var presentation = other.whitespacePresentation
        presentation.visibleWhitespace.style.size = 26
        try other.setWhitespacePresentation(presentation)
        XCTAssertEqual(try session.snapshot().definition(for: .visibleWhitespace)?.properties[.characterSize]?.declared, .float(26))
        XCTAssertThrowsError(try session.edit(key: .visibleWhitespace, expected: beforeExternal.identity,
            mutation: .setDeclaration(.characterSize, .float(99))))
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 26)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(session.undoManager.canRedo)
    }

    func testFailedWhitespaceUndoReportsTheSettingsErrorInTheOrdinaryEditor() throws {
        let (directory, configuration) = fixture()
        let coordinator = EVStyleEditorCoordinator()
        coordinator.showCode(configuration: configuration, preferredStyle: .visibleWhitespace, sender: nil)
        defer { coordinator.close() }
        let editor = try XCTUnwrap(coordinator.styleWindow?.contentViewController as? EVStyleEditorViewController)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(24)))
        let undo = try XCTUnwrap(coordinator.styleWindow?.undoManager)
        XCTAssertTrue(undo.canUndo)
        try FileManager.default.removeItem(at: directory)
        try Data("obstruction".utf8).write(to: directory)
        undo.undo()
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style.size, 24)
        XCTAssertFalse(editor.inspection.diagnostic.isEmpty)
        XCTAssertEqual(try Data(contentsOf: directory), Data("obstruction".utf8))
    }

    func testWhitespaceDefinitionCannotBecomeAThemeParentOrAcceptUnsupportedMutations() throws {
        let (_, configuration) = fixture()
        let session = try EVCodeStyleSession(configuration: configuration)
        let snapshot = try session.snapshot()
        let whitespace = try XCTUnwrap(snapshot.definition(for: .visibleWhitespace))
        XCTAssertEqual(whitespace.key.namespace, .applicationSetting)
        XCTAssertEqual(whitespace.kind, .character)
        XCTAssertEqual(whitespace.capabilities, [.declarations])
        XCTAssertTrue(snapshot.legalParents(for: whitespace).isEmpty)
        for definition in snapshot.definitions {
            XCTAssertFalse(snapshot.legalParents(for: definition).contains { $0.key == .visibleWhitespace })
        }
        let before = configuration.whitespacePresentation.visibleWhitespace.style
        let invalidMutations: [EVStyleMutation] = [.clearParent, .setDisplayName("Other"),
            .setDeclaration(.paragraphFirstLineIndent, .float(4))]
        for mutation in invalidMutations {
            XCTAssertThrowsError(try session.edit(key: .visibleWhitespace,
                expected: session.snapshot().identity, mutation: mutation))
        }
        XCTAssertEqual(configuration.whitespacePresentation.visibleWhitespace.style, before)
        XCTAssertFalse(session.undoManager.canUndo)
    }

    private func descendants(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }
        return [view] + view.subviews.flatMap { descendants($0) }
    }
}
