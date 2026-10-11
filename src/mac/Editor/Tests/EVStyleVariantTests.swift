import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleVariantTests: XCTestCase {
    private let lineNumber = EVStyleTestFixtures.block("Builtin line number")

    func testForeignParentsUseDocumentIconsAndRejectCyclesWithoutChangingTheChoiceOrHistory() throws {
        let configuration = isolatedConfiguration()
        try EVStyleTestFixtures.configure(configuration, format: .markdown, declarations: [
            (lineNumber, .characterSize, .float(17)),
        ])
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (lineNumber, .characterSize, nil),
        ])
        let code = try EVCodeStyleSession(configuration: configuration)
        let markdown = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        let editor = EVStyleEditorViewController()
        editor.retarget(settingsSession: code)
        editor.selectStyle(lineNumber)
        let popup = try parentPopup(editor)
        let name = try XCTUnwrap(code.snapshot().definition(for: lineNumber)?.name)
        XCTAssertEqual(Array(popup.itemTitles.prefix(2)), ["Plain Text: \(name)", "Markdown: \(name)"])
        XCTAssertTrue(popup.itemArray.dropFirst(2).first?.isSeparatorItem == true)
        for item in popup.itemArray.prefix(2) {
            XCTAssertEqual(item.image?.accessibilityDescription, "Document stylesheet")
            XCTAssertEqual(item.image?.size, NSSize(width: 16, height: 16))
            XCTAssertTrue(item.image?.isTemplate == true)
        }
        let original = try configuration.codeStyleSheet()
        choose("Markdown: \(name)", in: popup)
        XCTAssertEqual(try code.snapshot().definition(for: lineNumber)?.parentVariant, .markdown)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(17))
        let inherited = try configuration.codeStyleSheet()
        XCTAssertNotEqual(inherited, original)
        code.undoManager.undo()
        XCTAssertEqual(try configuration.codeStyleSheet(), original)
        code.undoManager.redo()
        XCTAssertEqual(try configuration.codeStyleSheet(), inherited)

        editor.retarget(settingsSession: markdown)
        XCTAssertEqual(editor.inspection.selectedStyleKey, lineNumber)
        let previousChoice = popup.titleOfSelectedItem
        let before = try ["text", "markdown", "code"].map { try configuration.styleDefaults(named: $0) }
        var cycle: String?
        editor.onInheritanceCycle = { cycle = $0 }
        choose("Code: \(name)", in: popup)
        XCTAssertEqual(cycle, "\"Markdown: \(name)\" can't depend on \"Code: \(name)\" because that creates a dependency cycle.")
        XCTAssertEqual(popup.titleOfSelectedItem, previousChoice)
        XCTAssertEqual(try ["text", "markdown", "code"].map { try configuration.styleDefaults(named: $0) }, before)
        XCTAssertFalse(markdown.undoManager.canUndo)

        try markdown.edit(key: lineNumber, expected: markdown.snapshot().identity,
                          mutation: .setDeclaration(.characterSize, .float(19)))
        editor.retarget(settingsSession: code)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(19),
                       "Changing the parent variant must refresh an existing dependent session")
        XCTAssertTrue(code.undoManager.canUndo, "A dependency-only refresh must retain the child's settings history")
    }

    func testBaseParagraphOffersOnlyForeignParentsAndNone() throws {
        let configuration = isolatedConfiguration()
        try EVStyleTestFixtures.configure(configuration, format: .plainText, declarations: [
            (.baseParagraph, .characterSize, .float(21)),
        ])
        let session = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        let editor = EVStyleEditorViewController()
        editor.retarget(settingsSession: session)
        editor.selectStyle(EVStyleKey.baseParagraph)
        let popup = try parentPopup(editor)
        let name = try XCTUnwrap(session.snapshot().definition(for: .baseParagraph)?.name)
        XCTAssertEqual(popup.itemTitles.filter { !$0.isEmpty }, ["Plain Text: \(name)", "Code: \(name)", "None"])
        XCTAssertTrue(popup.isEnabled)
        choose("Plain Text: \(name)", in: popup)
        XCTAssertEqual(try session.snapshot().definition(for: .baseParagraph)?.parentVariant, .plainText)
        let beforeClear = try session.snapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared
        XCTAssertNotNil(beforeClear, "Choosing a parent must retain the child's explicit declarations")
        let sizeOverride = try XCTUnwrap(descendants(editor.view).compactMap { $0 as? NSButton }.first {
            $0.tag == Int(EVStyleProperty.characterSize.rawValue) && $0.toolTip == "Override inherited"
        })
        XCTAssertTrue(sizeOverride.isEnabled)
        sizeOverride.performClick(nil)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(21))
        XCTAssertFalse(editor.inspection.declaredProperties.contains(.characterSize))
        choose("None", in: popup)
        XCTAssertNil(try session.snapshot().definition(for: .baseParagraph)?.parentVariant)
        XCTAssertEqual(popup.titleOfSelectedItem, "None")
    }

    func testSharedChromeDeclarationsKeepEachVariantsOwnParent() throws {
        let configuration = isolatedConfiguration()
        let status = EVStyleTestFixtures.block("Status bar")
        let markdown = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        let beforeText = try configuration.styleDefaults(named: "text")
        let beforeCode = try configuration.styleDefaults(named: "code")
        try markdown.edit(key: status, expected: markdown.snapshot().identity,
                          mutation: .setParentVariant(.plainText))
        XCTAssertEqual(try configuration.styleDefaults(named: "text"), beforeText)
        XCTAssertEqual(try configuration.styleDefaults(named: "code"), beforeCode)
        let code = try EVCodeStyleSession(configuration: configuration)
        try code.edit(key: status, expected: code.snapshot().identity,
                      mutation: .setParentVariant(.markdown))
        try markdown.edit(key: status, expected: markdown.snapshot().identity,
                          mutation: .setDeclaration(.characterSize, .float(27)))
        let text = try EVThemeStyleSession(configuration: configuration, format: .plainText)
        XCTAssertNil(try text.snapshot().definition(for: status)?.parentVariant)
        XCTAssertEqual(try markdown.snapshot().definition(for: status)?.parentVariant, .plainText)
        XCTAssertEqual(try code.snapshot().definition(for: status)?.parentVariant, .markdown)
        for snapshot in [try text.snapshot(), try markdown.snapshot(), try code.snapshot()] {
            XCTAssertEqual(snapshot.definition(for: status)?.properties[.characterSize]?.declared, .float(27))
        }
        try markdown.edit(key: status, expected: markdown.snapshot().identity,
                          mutation: .clearDeclaration(.characterSize))
        let freshMarkdown = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        XCTAssertEqual(try markdown.snapshot().definition(for: status)?.properties[.characterSize]?.effective,
                       try freshMarkdown.snapshot().definition(for: status)?.properties[.characterSize]?.effective,
                       "Clearing shared declarations must refresh the edited session's own foreign parent context")
        try code.edit(key: status, expected: code.snapshot().identity,
                      mutation: .setDeclaration(.characterSize, .float(31)))
        try code.edit(key: status, expected: code.snapshot().identity,
                      mutation: .clearDeclaration(.characterSize))
        XCTAssertEqual(try code.snapshot().definition(for: status)?.properties[.characterSize]?.effective,
                       try text.snapshot().definition(for: status)?.properties[.characterSize]?.effective,
                       "Clearing global Code declarations must refresh its foreign dependency context too")
    }

    func testDependentOpenDocumentRefreshesWithoutChangingSourceOrItsHistory() throws {
        let configuration = isolatedConfiguration()
        try EVStyleTestFixtures.configure(configuration, format: .plainText, declarations: [
            (lineNumber, .characterSize, .float(15)),
        ])
        let markdown = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        try markdown.edit(key: lineNumber, expected: markdown.snapshot().identity,
                          mutation: .clearDeclaration(.characterSize))
        try markdown.edit(key: lineNumber, expected: markdown.snapshot().identity,
                          mutation: .setParentVariant(.plainText))
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Heading\n\nBody".utf8)
        try backend.read(source: source, typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: lineNumber)?.properties[.characterSize]?.effective, .float(15))
        let text = try EVThemeStyleSession(configuration: configuration, format: .plainText)
        try text.edit(key: lineNumber, expected: text.snapshot().identity,
                      mutation: .setDeclaration(.characterSize, .float(22)))
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: lineNumber)?.properties[.characterSize]?.effective, .float(22))
        text.undoManager.undo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: lineNumber)?.properties[.characterSize]?.effective, .float(15))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
    }

    private func choose(_ title: String, in popup: NSPopUpButton) {
        XCTAssertNotNil(popup.item(withTitle: title))
        popup.selectItem(withTitle: title)
        XCTAssertTrue(popup.sendAction(popup.action, to: popup.target))
    }

    private func parentPopup(_ editor: EVStyleEditorViewController) throws -> NSPopUpButton {
        return try XCTUnwrap(descendants(editor.view).compactMap { $0 as? NSPopUpButton }
            .first { $0.accessibilityLabel() == "Based on style" })
    }

    private func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }

    private func isolatedConfiguration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-variant-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }
}
