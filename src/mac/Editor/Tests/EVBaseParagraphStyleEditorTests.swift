import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBaseParagraphStyleEditorTests: XCTestCase {
    func testBaseParagraphNextStyleIsFixedAcrossGeneratedAndNativeStyles() throws {
        for (type, source, headingID) in [
            ("public.markdown", "Text", "Heading1"),
        ] {
            let configuration = isolatedConfiguration()
            let backend = EVCoreDocumentBackend(configuration: configuration)
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            defer { withExtendedLifetime(surface) {} }
            surface.loadViewIfNeeded()
            let editor = EVStyleEditorViewController()
            editor.themeStore = EVThemeStore(configuration: configuration)
            editor.retarget(document: surface, styleKey: .baseParagraph)
            let before = try backend.recoverySnapshot()
            let snapshot = try backend.styleSheetSnapshot()
            let root = try XCTUnwrap(snapshot.definition(for: .baseParagraph))
            XCTAssertNil(root.nextStyleID)
            XCTAssertFalse(root.capabilities.contains(.nextStyle))
            try assertNextStyleLocked(in: editor)

            let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: headingID))
            XCTAssertFalse(editor.setFollowingStyleForTesting(heading))
            XCTAssertEqual(try backend.recoverySnapshot(), before)
            XCTAssertEqual(try backend.styleSheetSnapshot().identity, snapshot.identity)
            XCTAssertFalse(surface.canUndo)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))

            editor.selectStyle(heading)
            XCTAssertTrue(try nextStylePopup(in: editor).isEnabled)
            editor.selectStyle(EVStyleKey.baseParagraph)
            try assertNextStyleLocked(in: editor)
        }
    }

    func testGlobalCodeBaseParagraphNextStyleIsSameStyle() throws {
        let configuration = isolatedConfiguration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(settingsSession: session)
        editor.selectStyle(EVStyleKey.baseParagraph)
        try assertNextStyleLocked(in: editor)
        XCTAssertFalse(session.undoManager.canUndo)
    }

    func testEveryBaseParagraphOverrideIsLockedWhileValuesRemainEditable() throws {
        for (type, source) in [
            ("public.markdown", "Text"),
        ] {
            let configuration = isolatedConfiguration()
            let backend = EVCoreDocumentBackend(configuration: configuration)
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            defer { withExtendedLifetime(surface) {} }
            surface.loadViewIfNeeded()
            let editor = EVStyleEditorViewController()
            editor.themeStore = EVThemeStore(configuration: configuration)
            editor.retarget(document: surface, styleKey: .baseParagraph)
            let before = try backend.recoverySnapshot()
            let snapshot = try backend.styleSheetSnapshot()
            let root = try XCTUnwrap(snapshot.definition(for: .baseParagraph))
            XCTAssertNil(root.properties[.characterBackground]?.declared)

            try assertOverridesLockedAndValuesEditable(in: editor)
            XCTAssertEqual(try backend.styleSheetSnapshot(), snapshot)
            XCTAssertEqual(try backend.recoverySnapshot(), before)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            XCTAssertFalse(surface.canUndo)
            XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        }
    }

    func testGlobalCodeBaseParagraphOverrideClicksLeaveSettingsAndHistoryUnchanged() throws {
        let configuration = isolatedConfiguration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(settingsSession: session)
        editor.selectStyle(EVStyleKey.baseParagraph)
        let snapshot = try session.snapshot()
        let persisted = try configuration.codeStyleSheet()

        try assertOverridesLockedAndValuesEditable(in: editor)
        XCTAssertEqual(try session.snapshot(), snapshot)
        XCTAssertEqual(try configuration.codeStyleSheet(), persisted)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
    }

    private func assertOverridesLockedAndValuesEditable(in editor: EVStyleEditorViewController) throws {
        let overrides = descendants(of: editor.view).compactMap { $0 as? NSButton }
            .filter { $0.toolTip == "Override inherited" }
        let expectedProperties = Set((EVStyleProperty.characterProperties + EVStyleProperty.paragraphProperties + EVStyleProperty.blockProperties)
            .filter { $0 != .characterLanguage }.map { Int($0.rawValue) })
        XCTAssertEqual(Set(overrides.map(\.tag)), expectedProperties,
                       "Every character and paragraph override must be covered")
        XCTAssertEqual(overrides.count, expectedProperties.count)
        for checkbox in overrides {
            let label = checkbox.accessibilityLabel() ?? "Override"
            XCTAssertEqual(checkbox.state, .on, label)
            XCTAssertFalse(checkbox.isEnabled, label)
            checkbox.performClick(nil)
            XCTAssertEqual(checkbox.state, .on, "\(label) cannot be unchecked")
        }

        for (tab, labels) in [
            (EVStyleEditorTab.character, ["Font family", "Font face", "Fallback fonts", "Size", "Bold",
                "Italic", "Underline", "Strikethrough", "Text color", "Background color", "Tracking",
                "Character direction", "Adjust size", "Adjust tracking"]),
            (EVStyleEditorTab.paragraph, ["Paragraph alignment", "Paragraph direction", "Start indent",
                "End indent", "First line", "Line spacing kind",
                "Adjust start indent", "Adjust end indent", "Adjust first line"]),
            (EVStyleEditorTab.block, ["Margin top", "Margin right", "Margin bottom", "Margin left",
                "Padding top", "Padding right", "Padding bottom", "Padding left", "Block background",
                "Border top weight", "Border right weight", "Border bottom weight", "Border left weight"]),
        ] {
            editor.selectTab(tab)
            for label in labels {
                let control = try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? NSControl }
                    .first { $0.accessibilityLabel() == label }, label)
                XCTAssertTrue(control.isEnabled, "\(label) remains editable")
                if let field = control as? NSTextField {
                    XCTAssertTrue(field.isEditable, label)
                    XCTAssertFalse(field.stringValue.isEmpty, "\(label) displays its effective root value")
                }
            }
        }
    }

    private func assertNextStyleLocked(in editor: EVStyleEditorViewController) throws {
        let popup = try nextStylePopup(in: editor)
        XCTAssertEqual(popup.titleOfSelectedItem, "Same Style")
        XCTAssertEqual(popup.numberOfItems, 1)
        XCTAssertFalse(popup.isEnabled)
        let next = try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "Edit next paragraph style" })
        XCTAssertFalse(next.isEnabled)
    }

    private func nextStylePopup(in editor: EVStyleEditorViewController) throws -> NSPopUpButton {
        try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? NSPopUpButton }
            .first { $0.accessibilityLabel() == "Following paragraph style" })
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    private func isolatedConfiguration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-base-paragraph-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }
}
