import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVColorPanelTargetTests: XCTestCase {
    func testDirectTypographyTakesThePanelFromAnActiveStyleWell() throws {
        let fixture = try makeFixture()
        defer { fixture.close() }
        let panel = NSColorPanel.shared
        let styleBefore = try fixture.styleBackend.styleSheetSnapshot()
        let directBefore = try fixture.directBackend.serializedSource(typeName: EVDocument.htmlType)
        fixture.well.showMoreColors()
        XCTAssertTrue(fixture.well.isActive)

        EVTypographyPanels.shared.showColors(for: fixture.directSurface, highlight: false)
        XCTAssertFalse(fixture.well.isActive,
                       "The direct panel must disconnect the previously active style well before seeding its color")
        XCTAssertEqual(try fixture.styleBackend.styleSheetSnapshot(), styleBefore)
        XCTAssertEqual(try fixture.directBackend.serializedSource(typeName: EVDocument.htmlType), directBefore)
        panel.color = NSColor(deviceRed: 0.25, green: 0.5, blue: 0.75, alpha: 1)

        XCTAssertEqual(try fixture.styleBackend.styleSheetSnapshot(), styleBefore,
                       "A direct text color gesture must not edit a named style in another document")
        XCTAssertNotEqual(try fixture.directBackend.serializedSource(typeName: EVDocument.htmlType), directBefore)
        let direct = try XCTUnwrap(fixture.directSurface.session).selectedTypography()
        let color = try XCTUnwrap(direct.foreground)
        XCTAssertEqual(color.red, 0.25, accuracy: 0.001)
        XCTAssertEqual(color.green, 0.5, accuracy: 0.001)
        XCTAssertEqual(color.blue, 0.75, accuracy: 0.001)
        _ = try fixture.directSurface.session?.undo()
        XCTAssertEqual(try fixture.directBackend.serializedSource(typeName: EVDocument.htmlType), directBefore)
        XCTAssertFalse(fixture.directSurface.canUndo)
    }

    func testStyleWellTakesThePanelFromDirectTypographyWithoutEditingItsSelection() throws {
        let fixture = try makeFixture()
        defer { fixture.close() }
        let panel = NSColorPanel.shared
        EVTypographyPanels.shared.showColors(for: fixture.directSurface, highlight: false)
        let directBefore = try fixture.directBackend.recoverySnapshot()
        let styleBefore = try fixture.styleBackend.styleSheetSnapshot()

        fixture.well.showMoreColors()
        XCTAssertTrue(fixture.well.isActive)
        XCTAssertEqual(try fixture.directBackend.recoverySnapshot(), directBefore,
                       "Seeding the style color must not invoke the panel's previous direct-formatting target")
        XCTAssertEqual(try fixture.styleBackend.styleSheetSnapshot(), styleBefore)
        panel.color = NSColor(deviceRed: 0.25, green: 0.5, blue: 0.75, alpha: 1)

        XCTAssertEqual(try fixture.directBackend.recoverySnapshot(), directBefore,
                       "Only the active style well owns the next panel gesture")
        XCTAssertFalse(fixture.directSurface.canUndo)
        let changed = try fixture.styleBackend.styleSheetSnapshot()
        let value = try XCTUnwrap(changed.definition(for: .baseParagraph)?
            .properties[.characterForeground]?.declared)
        guard case let .color(color) = value else { return XCTFail("The style color must be declared") }
        XCTAssertEqual(color.red, 0.25, accuracy: 0.001)
        XCTAssertEqual(color.green, 0.5, accuracy: 0.001)
        XCTAssertEqual(color.blue, 0.75, accuracy: 0.001)
        _ = try fixture.styleSurface.session?.undo()
        XCTAssertEqual(try fixture.styleBackend.styleSheetSnapshot().definition(for: .baseParagraph),
                       styleBefore.definition(for: .baseParagraph))
    }

    @MainActor
    private struct Fixture {
        let styleBackend: EVCoreDocumentBackend
        let styleSurface: EVEditorSurfaceController
        let directBackend: EVCoreDocumentBackend
        let directSurface: EVEditorSurfaceController
        let window: NSWindow
        let well: EVStyleColorWell

        func close() {
            well.dismissColorControls()
            EVStyleColorWell.deactivatePanelOwner()
            NSColorPanel.shared.setTarget(nil)
            NSColorPanel.shared.setAction(nil)
            NSColorPanel.shared.orderOut(nil)
            window.orderOut(nil)
        }
    }

    private func makeFixture() throws -> Fixture {
        EVStyleColorWell.deactivatePanelOwner()
        NSColorPanel.shared.setTarget(nil)
        NSColorPanel.shared.setAction(nil)
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-color-panel-target-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let styleBackend = EVCoreDocumentBackend(configuration: configuration)
        try styleBackend.read(source: Data("Style preview".utf8), typeName: EVDocument.markdownType)
        let styleSurface = try XCTUnwrap(styleBackend.makeEditorSurface() as? EVEditorSurfaceController)
        styleSurface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: styleSurface, styleKey: .baseParagraph)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground,
            value: .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1))))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 650),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        editor.view.layoutSubtreeIfNeeded()
        let well = try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? EVStyleColorWell }
            .first { $0.accessibilityLabel() == "Text color" })
        XCTAssertTrue(well.isEnabled)

        let directBackend = EVCoreDocumentBackend(configuration: configuration)
        try directBackend.read(source: Data("<p>Selected words</p><!--keep-->".utf8), typeName: EVDocument.htmlType)
        let directSurface = try XCTUnwrap(directBackend.makeEditorSurface() as? EVEditorSurfaceController)
        directSurface.loadViewIfNeeded()
        directSurface.perform(menuCommand: .selectAll, sender: nil)
        XCTAssertTrue(directSurface.canEditTypography)
        return Fixture(styleBackend: styleBackend, styleSurface: styleSurface,
                       directBackend: directBackend, directSurface: directSurface, window: window, well: well)
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }
}
