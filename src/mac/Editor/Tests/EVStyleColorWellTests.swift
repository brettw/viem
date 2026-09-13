import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleColorWellTests: XCTestCase {
    func testOpeningNativePanelShowsCustomColorWithoutEditingOrAddingUndo() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let original = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]
        let custom = EVStyleColor(red: 0.12345679, green: 0.25, blue: 0.5, alpha: 0.4)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(custom)))
        let before = try backend.styleSheetSnapshot()
        let bytes = try backend.serializedSource(typeName: EVDocument.htmlType)
        let well = try colorWell("Text color", in: editor.view)
        NSColorPanel.shared.color = .yellow

        try openColorPanel(well)
        try assertColor(NSColorPanel.shared.color, equals: custom)
        try assertColor(well.color, equals: custom)
        try openColorPanel(well)
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), bytes)

        well.dismissColorControls()
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground], original)
        XCTAssertFalse(surface.canUndo, "Opening the native panel adds no undo step or color-space round-trip edit")
    }

    func testNativePanelSelectionCommitsOnceAndReopeningShowsCommittedColor() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let before = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]
        XCTAssertFalse(surface.canUndo)
        let well = try colorWell("Text color", in: editor.view)
        try openColorPanel(well)
        let expected = EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1)

        NSColorPanel.shared.color = expected.appKitColor

        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground]?.declared, .color(expected))
        try assertColor(well.color, equals: expected)
        XCTAssertTrue(surface.canUndo)
        well.dismissColorControls()
        NSColorPanel.shared.color = .red
        let committed = try backend.styleSheetSnapshot()
        try openColorPanel(well)
        try assertColor(NSColorPanel.shared.color, equals: expected)
        XCTAssertEqual(try backend.styleSheetSnapshot(), committed)
        well.dismissColorControls()
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterForeground], before)
        XCTAssertFalse(surface.canUndo, "One native panel choice is one undo step")
    }

    func testSwitchingFromTextToBackgroundColorSeedsCurrentColorWithoutEditingText() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let foreground = EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1)
        let background = EVStyleColor(red: 0, green: 0.5, blue: 0.25, alpha: 0.5)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(foreground)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterBackground, value: .color(background)))
        let textWell = try colorWell("Text color", in: editor.view)
        let backgroundWell = try colorWell("Background color", in: editor.view)
        try openColorPanel(textWell)
        let before = try backend.styleSheetSnapshot()

        try openColorPanel(backgroundWell)

        XCTAssertFalse(textWell.isActive)
        try assertColor(NSColorPanel.shared.color, equals: background)
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
        let chosen = EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 0.5)
        NSColorPanel.shared.color = chosen.appKitColor
        let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(for: .baseParagraph))
        XCTAssertEqual(definition.properties[.characterForeground]?.declared, .color(foreground))
        XCTAssertEqual(definition.properties[.characterBackground]?.declared, .color(chosen))
    }

    func testRTFPanelNormalizesColorAndOffersTransparentBackground() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.rtfType, source: #"{\rtf1 Text}"#)
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let well = try colorWell("Background color", in: editor.view)
        try openColorPanel(well)
        XCTAssertTrue(NSColorPanel.shared.showsAlpha)

        NSColorPanel.shared.color = NSColor(srgbRed: 0.75, green: 0.25, blue: 0.125, alpha: 0.4)

        let expected = EVStyleColor(red: 191 / 255, green: 64 / 255, blue: 32 / 255, alpha: 1)
        try assertColor(well.color, equals: expected)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterBackground]?.declared, .color(expected))
        well.dismissColorControls()
        try openColorPanel(well)
        try assertColor(NSColorPanel.shared.color, equals: expected)

        NSColorPanel.shared.color = NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0)

        let transparent = EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterBackground]?.declared, .color(transparent))
        XCTAssertEqual(editor.inspection.diagnostic, "")
        well.dismissColorControls()
        try openColorPanel(well)
        try assertColor(NSColorPanel.shared.color, equals: transparent)
    }

    func testFirstClickOnInheritedWellEnablesOverrideAndOpensNativePanel() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.markdownType, source: "# Heading\n\nBody")
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
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
        XCTAssertTrue(well.isActive)
        XCTAssertTrue(NSColorPanel.shared.isVisible)
        let transparent = EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0)
        try assertColor(NSColorPanel.shared.color, equals: transparent)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterBackground]?.declared,
            .color(transparent))
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
    }

    func testStyleAndReadOnlyTransitionsDisconnectNativePanel() throws {
        let (backend, surface, editor, window) = try makeEditor(typeName: EVDocument.markdownType, source: "# Heading\n\nBody")
        defer { closeColorPanel(); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let well = try colorWell("Text color", in: editor.view)
        try openColorPanel(well)

        editor.selectStyle(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))

        XCTAssertFalse(well.isActive)
        let afterRetarget = try backend.styleSheetSnapshot()
        NSColorPanel.shared.color = .magenta
        XCTAssertEqual(try backend.styleSheetSnapshot(), afterRetarget,
                       "A panel opened for the previous style must not edit the newly selected style")
        editor.selectStyle(EVStyleKey.baseParagraph)
        let custom = EVStyleColor(red: 0.12345679, green: 0.8765432, blue: 0.33333334, alpha: 0.44444445)
        XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(custom)))
        try openColorPanel(well)
        try assertColor(NSColorPanel.shared.color, equals: custom)
        let owner = try XCTUnwrap(well.target as? EVCompactStyleControls)
        let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(for: .baseParagraph))
        let readOnly = EVStyleDefinition(key: definition.key, name: definition.name, kind: definition.kind,
            origin: definition.origin, flags: definition.flags, capabilities: [], parentID: definition.parentID,
            nextStyleID: definition.nextStyleID, properties: definition.properties)

        owner.configure(readOnly)

        XCTAssertFalse(well.isActive)
        XCTAssertFalse(well.isEnabled)
        let afterReadOnly = try backend.styleSheetSnapshot()
        NSColorPanel.shared.color = .cyan
        well.showColorPanel()
        XCTAssertFalse(well.isActive)
        XCTAssertEqual(try backend.styleSheetSnapshot(), afterReadOnly)
    }

    private func makeEditor(typeName requestedTypeName: String? = nil, source: String = "<p>Text</p>") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, NSWindow) {
        closeColorPanel()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-color-panel-\(UUID().uuidString)")
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
    private func openColorPanel(_ well: EVStyleColorWell) throws {
        XCTAssertTrue(well.sendAction(try XCTUnwrap(well.pulldownAction), to: well.pulldownTarget))
        XCTAssertTrue(well.isActive)
        XCTAssertTrue(NSColorPanel.shared.isVisible)
        XCTAssertFalse(NSColorPanel.shared.isContinuous)
    }
    private func assertColor(_ color: NSColor, equals expected: EVStyleColor,
                             file: StaticString = #filePath, line: UInt = #line) throws {
        let rgb = try XCTUnwrap(color.usingColorSpace(.sRGB), file: file, line: line)
        XCTAssertEqual(rgb.redComponent, CGFloat(expected.red), accuracy: 0.000001, file: file, line: line)
        XCTAssertEqual(rgb.greenComponent, CGFloat(expected.green), accuracy: 0.000001, file: file, line: line)
        XCTAssertEqual(rgb.blueComponent, CGFloat(expected.blue), accuracy: 0.000001, file: file, line: line)
        XCTAssertEqual(rgb.alphaComponent, CGFloat(expected.alpha), accuracy: 0.000001, file: file, line: line)
    }
    private func closeColorPanel() {
        EVStyleColorWell.deactivatePanelOwner()
        NSColorPanel.shared.setTarget(nil)
        NSColorPanel.shared.setAction(nil)
        NSColorPanel.shared.orderOut(nil)
    }
}
