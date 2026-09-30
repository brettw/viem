import AppKit
import XCTest
import ViemAppShell
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

    func testModelessEditorSharesCompactControlsAndHasIndependentUndo() throws {
        let (_, configuration) = fixture()
        let editor = EVVisibleWhitespaceStyleEditor(configuration: configuration)
        defer { editor.close() }
        editor.showWindow(nil)
        let window = try XCTUnwrap(editor.window)
        XCTAssertTrue(window.undoManager === editor.session.undoManager)
        XCTAssertEqual(window.title, "Visible whitespace")
        XCTAssertNil(window.sheetParent)
        let views = descendants(window.contentView)
        XCTAssertNotNil(views.first { $0.accessibilityLabel() == "Font family" })
        XCTAssertNotNil(views.first { $0.accessibilityLabel() == "Live style preview" })
        let language = try XCTUnwrap(views.first { $0.accessibilityLabel() == "Override marker language" } as? NSButton)
        language.performClick(nil)
        XCTAssertEqual(editor.session.style.language, "en")
        language.performClick(nil)
        XCTAssertNil(editor.session.style.language)
        let slant = try XCTUnwrap(views.first { $0.accessibilityLabel() == "Marker slant" } as? NSPopUpButton)
        slant.selectItem(at: 3)
        slant.sendAction(slant.action, to: slant.target)
        XCTAssertEqual(editor.session.style.slant, .oblique)
        slant.selectItem(at: 0)
        slant.sendAction(slant.action, to: slant.target)
        XCTAssertNil(editor.session.style.slant)
        let weight = try XCTUnwrap(views.first { $0.accessibilityLabel() == "Marker weight" } as? NSTextField)
        weight.stringValue = "450"
        weight.sendAction(weight.action, to: weight.target)
        XCTAssertEqual(editor.session.style.weight, 450)
        weight.stringValue = "0"
        weight.sendAction(weight.action, to: weight.target)
        XCTAssertEqual(editor.session.style.weight, 450)
        XCTAssertEqual(weight.stringValue, "450")
        window.contentView?.layoutSubtreeIfNeeded()
        XCTAssertTrue(window.contentLayoutRect.contains(weight.convert(weight.bounds, to: nil)))
    }

    private func descendants(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }
        return [view] + view.subviews.flatMap { descendants($0) }
    }
}
