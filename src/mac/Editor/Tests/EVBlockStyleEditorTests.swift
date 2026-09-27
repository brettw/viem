import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBlockStyleEditorTests: XCTestCase {
    private let quote = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))

    func testContainerCategoryAndBlockTabKeepExistingSelectionRules() throws {
        let (_, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        XCTAssertEqual(editor.inspection.selectedKind, .quote)
        XCTAssertTrue(editor.inspection.paragraphTabEnabled)
        XCTAssertTrue(editor.inspection.blockTabEnabled)
        XCTAssertEqual(editor.inspection.blockPropertyCount, 17)
        let picker = try control(NSPopUpButton.self, "Style", editor)
        XCTAssertTrue(picker.itemTitles.contains("Container"))
        editor.selectTab(.block)
        XCTAssertEqual(editor.inspection.selectedTab, .block)
        let margin = try control(NSTextField.self, "Margin top", editor)
        XCTAssertFalse(margin.isHiddenOrHasHiddenAncestor)
        editor.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertEqual(editor.inspection.selectedTab, .block)
        XCTAssertTrue(editor.inspection.paragraphTabEnabled)
        editor.selectTab(.paragraph)
        XCTAssertTrue(margin.isHiddenOrHasHiddenAncestor)
        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertFalse(editor.inspection.blockTabEnabled)
        XCTAssertFalse(editor.inspection.paragraphTabEnabled)
        XCTAssertEqual(editor.inspection.selectedTab, .character)
    }

    func testBlockPropertiesUseInheritedControlsAndOneUndoGesture() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let session = try XCTUnwrap(surface.session)
        let before = try backend.styleSheetSnapshot().definition(for: quote)
        let originalSource = try backend.serializedSource(typeName: EVDocument.markdownType)
        editor.selectTab(.block)
        editor.beginContinuousStyleEditForTesting()
        XCTAssertTrue(editor.setPropertyForTesting(.blockMarginLeft, value: .float(-4)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(11)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderRightWidth, value: .float(3)))
        let purple = EVStyleColor(red: 0.7, green: 0.5, blue: 0.9, alpha: 0.6)
        XCTAssertTrue(editor.setPropertyForTesting(.blockBackground, value: .color(purple)))
        editor.endContinuousStyleEditForTesting()
        let changed = try backend.styleSheetSnapshot().definition(for: quote)
        XCTAssertEqual(changed?.properties[.blockPaddingLeft]?.declared, .float(11))
        XCTAssertEqual(changed?.properties[.blockBackground]?.declared, .color(purple))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), originalSource)
        _ = try session.undo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), before)
        _ = try session.redo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), changed)
        let toggle = try control(NSButton.self, "Override padding left", editor)
        XCTAssertEqual(toggle.state, .on)
        toggle.performClick(nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: quote)?.properties[.blockPaddingLeft]?.declared)
    }

    func testChangingToACharacterStyleEndsEditingInTheHiddenBlockTab() throws {
        let (_, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectStyle(EVStyleKey.baseParagraph)
        editor.selectTab(.block)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 650),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        defer { window.orderOut(nil) }
        editor.view.layoutSubtreeIfNeeded()
        let margin = try control(NSTextField.self, "Margin top", editor)
        XCTAssertTrue(window.makeFirstResponder(margin))
        XCTAssertNotNil(margin.currentEditor())
        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertEqual(editor.inspection.selectedTab, .character)
        XCTAssertNil(margin.currentEditor())
        XCTAssertFalse((window.firstResponder as? NSView)?.isHiddenOrHasHiddenAncestor == true)
    }

    func testNativeBlockBackgroundAndBorderPickersCommitColors() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        for (label, property) in [("Block background", EVStyleProperty.blockBackground), ("Border bottom color", .blockBorderBottomColor)] {
            let well = try control(EVStyleColorWell.self, label, editor)
            let toggle = try control(NSButton.self, "Override \(property.displayName.lowercased())", editor)
            if toggle.state == .off { toggle.performClick(nil) }
            XCTAssertTrue(well.isEnabled)
            well.color = NSColor(srgbRed: 0.25, green: 0.5, blue: 0.75, alpha: 0.5)
            XCTAssertTrue(well.sendAction(try XCTUnwrap(well.action), to: well.target))
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared,
                .color(EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 0.5)))
        }
    }

    func testNestedContainerPreviewUsesCoreBoxesAndRespondsToPaddingAndBorders() throws {
        let (_, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let background = EVStyleColor(red: 0.9, green: 0.8, blue: 1, alpha: 1)
        XCTAssertTrue(editor.setPropertyForTesting(.blockBackground, value: .color(background)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(9)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingTop, value: .float(4)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderLeftWidth, value: .float(2)))
        let initial = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
        XCTAssertNil(initial.blockPreviewError)
        XCTAssertGreaterThanOrEqual(initial.currentStyleLines.count, 3)
        let backgrounds = initial.boxes.filter(\.isBackground)
        XCTAssertGreaterThanOrEqual(backgrounds.count, 2, "Both nested quotation containers have a background")
        XCTAssertTrue(backgrounds.allSatisfy { $0.color == background })
        XCTAssertTrue(initial.boxes.contains { !$0.isBackground && abs($0.rect.width - 2) < 0.01 })
        let before = try XCTUnwrap(initial.currentStyleLines.last).origin.x
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(19)))
        let padded = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
        XCTAssertNil(padded.blockPreviewError)
        XCTAssertEqual(try XCTUnwrap(padded.currentStyleLines.last).origin.x - before, 20, accuracy: 0.1,
            "The nested paragraph receives ten additional points from each quote level")
    }

    func testCustomContainerStyleKeepsItsRoleAndCompatibleParents() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        try backend.read(source: Data("<blockquote><p>Quoted text</p></blockquote>".utf8), typeName: EVDocument.htmlType)
        editor.retarget(document: surface, styleKey: quote)
        XCTAssertTrue(editor.createStyle(kind: .quote), editor.inspection.diagnostic)
        let key = try XCTUnwrap(editor.inspection.selectedStyleKey)
        let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(for: key))
        XCTAssertEqual(definition.kind, .quote)
        XCTAssertEqual(definition.parentKey, quote)
        XCTAssertTrue(editor.inspection.parentChoices.contains(.baseParagraph))
        XCTAssertTrue(editor.inspection.parentChoices.contains(quote))
        XCTAssertFalse(editor.inspection.parentChoices.contains(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))))
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderTopWidth, value: .float(2)))
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: key)?.properties[.blockBorderTopWidth]?.declared, .float(2))
    }

    func testPaddingAndBorderNativeInputsRejectNegativeValuesButMarginsAcceptThem() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        for property in [EVStyleProperty.blockPaddingRight, .blockBorderBottomWidth, .blockMarginRight] {
            XCTAssertTrue(editor.setPropertyForTesting(property, value: .float(0)))
            let field = try control(NSTextField.self, property.displayName, editor)
            let original = try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared
            field.stringValue = "-3"
            field.delegate?.controlTextDidChange?(Notification(name: NSControl.textDidChangeNotification, object: field))
            if property == .blockMarginRight {
                XCTAssertFalse(editor.inspection.hasInvalidDraft)
                XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, .float(-3))
            } else {
                XCTAssertTrue(editor.inspection.hasInvalidDraft)
                XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, original)
            }
        }
    }

    private func makeEditor() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-block-editor-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("> First paragraph\n>\n> Second paragraph\n>\n> > Nested quote".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: quote)
        return (backend, surface, editor)
    }

    private func control<T: NSView>(_ type: T.Type, _ label: String, _ editor: EVStyleEditorViewController) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
        return try XCTUnwrap(descendants(editor.view).first { $0 is T && $0.accessibilityLabel() == label } as? T,
            "Missing \(label)")
    }
}
