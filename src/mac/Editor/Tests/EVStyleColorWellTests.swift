import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleColorWellTests: XCTestCase {
    func testOpeningPaletteDisplaysExactCustomColorAndSelectingCurrentDoesNotEdit() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let original = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]
        let custom = EVStyleColor(red: 0.12345679, green: 0.25, blue: 0.5, alpha: 0.4)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(custom)))
        let before = try backend.styleSheetSnapshot()
        let bytes = try backend.serializedSource(typeName: EVDocument.htmlType)
        let well = try colorWell("Text color", in: editor.view)
        try openPalette(well)
        let palette = try XCTUnwrap(well.paletteController)
        XCTAssertEqual(palette.currentColor, custom)
        XCTAssertEqual(palette.currentValueLabel.stringValue, "RGBA 0.12345679, 0.25, 0.5, 0.4")
        XCTAssertEqual(palette.currentSwatch.swatchColor, custom)
        XCTAssertEqual(palette.currentSwatch.state, .on)
        XCTAssertTrue(palette.choiceButtons.allSatisfy { $0.state == .off })
        XCTAssertNil(palette.transparentButton)
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
        XCTAssertTrue(surface.canUndo)
        palette.currentSwatch.performClick(nil)
        XCTAssertNil(well.palettePopover)
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), bytes)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground], original)
        XCTAssertFalse(surface.canUndo, "Opening and selecting the current color add no undo step")
    }

    func testPresetSelectionCommitsOnceAndReopeningChecksTheCommittedColor() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let before = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]
        let well = try colorWell("Text color", in: editor.view)
        try openPalette(well)
        let gray = try choice("Gray", in: XCTUnwrap(well.paletteController))
        gray.performClick(nil)
        XCTAssertNil(well.palettePopover)
        let expected = EVStyleColor(red: 128 / 255, green: 128 / 255, blue: 128 / 255, alpha: 1)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]?.declared, .color(expected))
        XCTAssertEqual(well.displayedColor, expected)
        XCTAssertTrue(surface.canUndo)
        try openPalette(well)
        let palette = try XCTUnwrap(well.paletteController)
        XCTAssertEqual(palette.currentValueLabel.stringValue, "#808080")
        XCTAssertEqual(try choice("Gray", in: palette).state, .on)
        XCTAssertEqual(palette.choiceButtons.filter { $0.state == .on }.count, 1)
        palette.currentSwatch.performClick(nil)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground], before)
        XCTAssertFalse(surface.canUndo, "One palette choice is one undo step; opening and current-color clicks add none")
    }

    func testRTFPaletteReflectsNormalizedNativeColorAndSupportsTransparentBackground() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.rtfType, source: #"{\rtf1 Text}"#)
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let well = try colorWell("Background color", in: editor.view)
        well.color = NSColor(deviceRed: 0.75, green: 0.25, blue: 0.125, alpha: 0.4)
        XCTAssertTrue(well.sendAction(try XCTUnwrap(well.action), to: well.target))
        let expected = EVStyleColor(red: 191 / 255, green: 64 / 255, blue: 32 / 255, alpha: 1)
        XCTAssertEqual(well.displayedColor, expected)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterBackground]?.declared, .color(expected))
        try openPalette(well)
        let palette = try XCTUnwrap(well.paletteController)
        XCTAssertEqual(palette.currentColor, expected)
        XCTAssertEqual(palette.currentValueLabel.stringValue, "#BF4020")
        try XCTUnwrap(palette.transparentButton).performClick(nil)
        XCTAssertEqual(well.displayedColor, EVStyleColorPaletteController.transparent)
        XCTAssertEqual(editor.inspection.diagnostic, "")
        try openPalette(well)
        XCTAssertEqual(well.paletteController?.transparentButton?.state, .on)
        XCTAssertEqual(well.paletteController?.currentValueLabel.stringValue, "#00000000")
        well.dismissColorControls()
    }

    func testFirstClickOnInheritedWellEnablesOverrideAndOpensPalette() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.markdownType, source: "# Heading\n\nBody")
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        let well = try colorWell("Background color", in: editor.view)
        XCTAssertFalse(well.isEnabled)
        let parent = try XCTUnwrap(editor.view.superview)
        let location = well.convert(NSPoint(x: well.bounds.midX, y: well.bounds.midY), to: nil)
        let hit = try XCTUnwrap(editor.view.hitTest(parent.convert(location, from: nil)))
        XCTAssertTrue(hit is EVInheritedStyleControl)
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location,
            modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil,
            eventNumber: 1, clickCount: 1, pressure: 1))
        hit.mouseDown(with: event)
        XCTAssertTrue(well.isEnabled)
        XCTAssertTrue(well.palettePopover?.isShown == true)
        XCTAssertEqual(well.paletteController?.currentColor, EVStyleColorPaletteController.transparent)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterBackground]?.declared,
            .color(EVStyleColorPaletteController.transparent))
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        well.dismissColorControls()
    }

    func testPaletteFollowsCommittedChangesAndClosesOnStyleOrReadOnlyTransition() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.markdownType, source: "# Heading\n\nBody")
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let well = try colorWell("Text color", in: editor.view)
        try openPalette(well)
        let popover = try XCTUnwrap(well.palettePopover)
        let custom = EVStyleColor(red: 0.12345679, green: 0.8765432, blue: 0.33333334, alpha: 0.44444445)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(custom)))
        XCTAssertTrue(well.palettePopover === popover)
        let palette = try XCTUnwrap(well.paletteController)
        XCTAssertEqual(palette.currentColor, custom)
        XCTAssertEqual(popover.contentSize, palette.preferredContentSize)
        palette.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(palette.view.bounds.contains(palette.view.convert(palette.currentValueLabel.bounds, from: palette.currentValueLabel)))
        editor.selectStyle(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
        XCTAssertNil(well.palettePopover)
        editor.selectStyle(EVStyleKey.baseParagraph)
        try openPalette(well)
        let owner = try XCTUnwrap(well.target as? EVCompactStyleControls)
        let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(for: .baseParagraph))
        let readOnly = EVStyleDefinition(key: definition.key, name: definition.name, kind: definition.kind,
            origin: definition.origin, flags: definition.flags, capabilities: [], parentID: definition.parentID,
            nextStyleID: definition.nextStyleID, properties: definition.properties)
        owner.configure(readOnly)
        XCTAssertNil(well.palettePopover)
        XCTAssertFalse(well.isEnabled)
        well.showPalette(nil)
        XCTAssertNil(well.palettePopover)
    }

    func testPaletteLayoutAndCurrentColorRemainVisibleInLightAndDarkAppearance() throws {
        let (_, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let custom = EVStyleColor(red: 0.12345679, green: 0.8765432, blue: 0.33333334, alpha: 0.44444445)
        XCTAssertTrue(editor.setPropertyForTesting(.characterBackground, value: .color(custom)))
        let well = try colorWell("Background color", in: editor.view)
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            window.appearance = try XCTUnwrap(NSAppearance(named: appearance))
            try openPalette(well)
            let palette = try XCTUnwrap(well.paletteController)
            palette.view.layoutSubtreeIfNeeded()
            XCTAssertEqual(palette.choiceButtons.count, 24)
            for control in descendants(of: palette.view).compactMap({ $0 as? NSControl }) {
                XCTAssertTrue(palette.view.bounds.contains(palette.view.convert(control.bounds, from: control)),
                    "All color choices and the exact current value fit in the compact popup")
            }
            XCTAssertEqual(palette.currentValueLabel.stringValue, EVStyleColorPaletteController.description(of: custom))
            if let directory = ProcessInfo.processInfo.environment["VIEM_STYLE_EDITOR_SCREENSHOT_DIR"] {
                let capture = try XCTUnwrap(palette.view.window?.contentView)
                capture.displayIfNeeded()
                let bitmap = try XCTUnwrap(capture.bitmapImageRepForCachingDisplay(in: capture.bounds))
                capture.cacheDisplay(in: capture.bounds, to: bitmap)
                let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                try png.write(to: URL(fileURLWithPath: directory, isDirectory: true)
                    .appendingPathComponent("style-color-palette-\(appearance.rawValue).png"))
            }
            well.dismissColorControls()
        }
    }

    private func makeEditor(typeName requestedTypeName: String? = nil, source: String = "<p>Text</p>") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, NSWindow) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-color-palette-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let typeName = requestedTypeName ?? EVDocument.htmlType
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: typeName)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: .baseParagraph)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 620),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        editor.view.layoutSubtreeIfNeeded()
        return (backend, surface, editor, window)
    }

    private func descendants(of root: NSView) -> [NSView] {
        root.subviews.flatMap { [$0] + descendants(of: $0) }
    }
    private func colorWell(_ label: String, in root: NSView) throws -> EVStyleColorWell {
        try XCTUnwrap(descendants(of: root).compactMap { $0 as? EVStyleColorWell }.first { $0.accessibilityLabel() == label })
    }
    private func choice(_ name: String, in palette: EVStyleColorPaletteController) throws -> NSButton {
        try XCTUnwrap(palette.choiceButtons.first { $0.accessibilityLabel() == name })
    }
    private func openPalette(_ well: EVStyleColorWell) throws {
        XCTAssertTrue(well.sendAction(try XCTUnwrap(well.pulldownAction), to: well.pulldownTarget))
        XCTAssertTrue(well.palettePopover?.isShown == true)
    }
}
