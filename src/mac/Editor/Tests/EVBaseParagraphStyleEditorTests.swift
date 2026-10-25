import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBaseParagraphStyleEditorTests: XCTestCase {

    func testEveryBaseParagraphOverrideIsLockedWhileValuesRemainEditable() throws {
        for (type, source) in [
            ("public.markdown", "Text"),
        ] {
            let configuration = try isolatedConfiguration()
            let backend = EVCoreDocumentBackend(configuration: configuration)
            try backend.readUntitled(source: Data(source.utf8), typeName: type)
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

    private func assertOverridesLockedAndValuesEditable(in editor: EVStyleEditorViewController) throws {
        let overrides = descendants(of: editor.view).compactMap { $0 as? NSButton }
            .filter { $0.toolTip == "Override inherited" }
        // Family, named face, base weight and variable axes share one override.
        // Language is preserved in the model but has no inspector control.
        let propertiesWithoutIndependentOverride: Set<EVStyleProperty> = [
            .characterLanguage, .characterWeight, .characterFontAxes, .characterFontFace,
            .paragraphLeadingIndent, .paragraphTrailingIndent,
        ]
        let expectedProperties = Set((EVStyleProperty.characterProperties + EVStyleProperty.paragraphProperties + EVStyleProperty.blockProperties)
            .filter { !propertiesWithoutIndependentOverride.contains($0) }.map { Int($0.rawValue) })
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
            (EVStyleEditorTab.paragraph, ["Paragraph alignment", "Paragraph direction", "First line",
                "Line spacing kind", "Adjust first line"]),
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

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    private func isolatedConfiguration() throws -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-base-paragraph-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        // These control tests need a family with available faces, independently
        // of the preset font and whether bundled fonts were registered by an app.
        for format in [EVSourceFormat.markdown, .code] {
            try EVStyleTestFixtures.configure(configuration, format: format, declarations: [
                (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
                (.baseParagraph, .characterFontAxes, .string("{}")),
            ])
        }
        return configuration
    }
}
