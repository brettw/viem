import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleColorWellTests: XCTestCase {

    func testBorderPanelRemainsBoundAcrossChoicesUntilItsTabIsHidden() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(in: editor.view); window.orderOut(nil); withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        let well = try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? EVStyleColorWell }
            .first { $0.tag == Int(EVStyleProperty.blockBorderLeftColor.rawValue) })
        try openColorPanel(well)
        for expected in [EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1),
                         EVStyleColor(red: 0.75, green: 0.25, blue: 0.5, alpha: 0.5)] {
            NSColorPanel.shared.color = expected.appKitColor
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?
                .properties[.blockBorderLeftColor]?.declared, .color(expected))
            XCTAssertTrue(well.isActive, "Refreshing the same visible attribute must retain the panel binding")
        }
        editor.selectTab(.character)
        XCTAssertFalse(well.isActive)
        let before = try backend.styleSheetSnapshot()
        NSColorPanel.shared.color = .red
        XCTAssertEqual(try backend.styleSheetSnapshot().identity, before.identity)
    }

    func testThemeBorderPanelRemainsBoundAcrossSavedChoices() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(in: editor.view); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let styles = try EVThemeStyleSession(configuration: backend.configuration, format: .markdown)
        editor.retarget(settingsSession: styles)
        editor.selectTab(.block)
        let well = try colorWell("Border left color", in: editor.view)
        let before = try styles.snapshot().definition(for: .baseParagraph)?.properties[.blockBorderLeftColor]
        try openColorPanel(well)
        for expected in [EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1),
                         EVStyleColor(red: 0.75, green: 0.25, blue: 0.5, alpha: 0.5)] {
            NSColorPanel.shared.color = expected.appKitColor
            XCTAssertEqual(try styles.snapshot().definition(for: .baseParagraph)?
                .properties[.blockBorderLeftColor]?.declared, .color(expected))
            XCTAssertTrue(well.isActive, "Saving a theme must retain the same visible attribute's binding")
        }
        NSColorPanel.shared.close()
        XCTAssertFalse(well.isActive)
        let committed = try styles.snapshot().identity
        NSColorPanel.shared.color = .red
        XCTAssertEqual(try styles.snapshot().identity, committed)
        styles.undoManager.undo()
        XCTAssertEqual(try styles.snapshot().definition(for: .baseParagraph)?
            .properties[.blockBorderLeftColor]?.declared,
            .color(EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1)))
        styles.undoManager.undo()
        XCTAssertEqual(try styles.snapshot().definition(for: .baseParagraph)?
            .properties[.blockBorderLeftColor], before)
        XCTAssertFalse(styles.undoManager.canUndo)
    }

    func testRetargetingSettingsDisconnectsThePreviousColorWell() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(in: editor.view); window.orderOut(nil); withExtendedLifetime(surface) {} }
        let first = try EVThemeStyleSession(configuration: backend.configuration, format: .markdown)
        editor.retarget(settingsSession: first)
        let well = try colorWell("Text color", in: editor.view)
        try openColorPanel(well)
        let second = try EVThemeStyleSession(configuration: backend.configuration, format: .plainText)
        editor.retarget(settingsSession: second)
        XCTAssertFalse(well.isActive)
        let before = try second.snapshot().identity
        NSColorPanel.shared.color = .red
        XCTAssertEqual(try second.snapshot().identity, before)
    }

    func testNativePanelSelectionCommitsOnceAndReopeningShowsCommittedColor() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { closeColorPanel(in: editor.view); window.orderOut(nil); withExtendedLifetime(surface) {} }
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

    private func makeEditor(typeName requestedTypeName: String? = nil, source: String = "Text") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, NSWindow) {
        closeColorPanel()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-color-panel-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let typeName = requestedTypeName ?? EVDocument.markdownType
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.readUntitled(source: Data(source.utf8), typeName: typeName)
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
    private func closeColorPanel(in root: NSView? = nil) {
        if let root {
            descendants(of: root).compactMap { $0 as? EVStyleColorWell }.forEach { $0.dismissColorControls() }
        }
        NSColorPanel.shared.setTarget(nil)
        NSColorPanel.shared.setAction(nil)
        NSColorPanel.shared.orderOut(nil)
    }
}
