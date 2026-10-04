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

    private let margins: [EVStyleProperty] = [.blockMarginLeft, .blockMarginRight, .blockMarginTop, .blockMarginBottom]
    private let weights: [EVStyleProperty] = [.blockBorderLeftWidth, .blockBorderRightWidth, .blockBorderTopWidth, .blockBorderBottomWidth]
    private let colors: [EVStyleProperty] = [.blockBorderLeftColor, .blockBorderRightColor, .blockBorderTopColor, .blockBorderBottomColor]
    private let paddings: [EVStyleProperty] = [.blockPaddingLeft, .blockPaddingRight, .blockPaddingTop, .blockPaddingBottom]

    func testBlockGridOrdersParametersAndSidesAndAlignsLockWithBackground() throws {
        let (_, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 650),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        defer { window.orderOut(nil) }
        editor.view.layoutSubtreeIfNeeded()
        var rows: [[NSRect]] = []
        for properties in [margins, weights, colors, paddings] {
            let frames = try properties.map { property in
                let field = try control(NSControl.self, property.displayName, editor)
                let frame = field.convert(field.bounds, to: editor.view)
                XCTAssertTrue(editor.view.bounds.contains(frame), "Every side must fit the minimum dialog width")
                XCTAssertGreaterThan(frame.width, 25)
                return frame
            }
            for (left, right) in zip(frames, frames.dropFirst()) {
                XCTAssertLessThan(left.maxX, right.minX)
                XCTAssertEqual(left.midY, right.midY, accuracy: 1)
            }
            rows.append(frames)
        }
        for pair in zip(rows, rows.dropFirst()) {
            for (above, below) in zip(pair.0, pair.1) {
                XCTAssertEqual(above.minX, below.minX, accuracy: 1)
                if editor.view.isFlipped { XCTAssertLessThan(above.maxY, below.minY) }
                else { XCTAssertGreaterThan(above.minY, below.maxY) }
            }
        }
        let lock = try control(NSButton.self, "Link block sides", editor)
        let background = try control(NSColorWell.self, "Block background", editor)
        let lockFrame = lock.convert(lock.bounds, to: editor.view)
        let backgroundFrame = background.convert(background.bounds, to: editor.view)
        XCTAssertEqual(lockFrame.midY, backgroundFrame.midY, accuracy: 1)
        XCTAssertGreaterThan(lockFrame.minX, rows[0].last!.maxX)
        XCTAssertNotNil(lock.image)
    }

    func testLockNormalizesFirstNonzeroValuesAsOneUndoGestureAndUnlockPreservesThem() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let session = try XCTUnwrap(surface.session)
        editor.selectTab(.block)
        for (properties, values) in [(margins, [0, -3, 8, 4]), (weights, [0, 2, 4, 0]), (paddings, [0, 0, 0, 0])] {
            for (property, value) in zip(properties, values) {
                XCTAssertTrue(editor.setPropertyForTesting(property, value: .float(Float(value))))
            }
        }
        for property in colors { XCTAssertTrue(editor.useInheritedForTesting(property)) }
        let blue = EVStyleColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 0.5)
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderRightColor, value: .color(blue)))
        let before = try backend.styleSheetSnapshot().definition(for: quote)
        let source = try backend.serializedSource(typeName: EVDocument.markdownType)
        let lock = try control(NSButton.self, "Link block sides", editor)
        XCTAssertEqual(lock.state, .off)
        lock.performClick(nil)
        let linked = try backend.styleSheetSnapshot().definition(for: quote)
        for (properties, value) in [(margins, EVStyleValue.float(-3)), (weights, .float(2)), (paddings, .float(0)), (colors, .color(blue))] {
            for property in properties { XCTAssertEqual(linked?.properties[property]?.declared, value) }
        }
        _ = try session.undo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), before)
        _ = try session.redo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), linked)
        lock.performClick(nil)
        XCTAssertEqual(lock.state, .off)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), linked)
        let bottom = try control(NSTextField.self, "Margin bottom", editor)
        bottom.stringValue = "7"
        bottom.delegate?.controlTextDidChange?(Notification(name: NSControl.textDidChangeNotification, object: bottom))
        let unlinked = try backend.styleSheetSnapshot().definition(for: quote)
        XCTAssertEqual(unlinked?.properties[.blockMarginBottom]?.declared, .float(7))
        XCTAssertEqual(unlinked?.properties[.blockMarginLeft]?.declared, .float(-3))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    }

    func testLinkedNativeFieldsSteppersColorsAndOverridesUpdateOnlyTheirParameter() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        for property in margins + weights + paddings { XCTAssertTrue(editor.setPropertyForTesting(property, value: .float(1))) }
        try control(NSButton.self, "Link block sides", editor).performClick(nil)
        let bottom = try control(NSTextField.self, "Margin bottom", editor)
        bottom.stringValue = "3"
        bottom.delegate?.controlTextDidChange?(Notification(name: NSControl.textDidChangeNotification, object: bottom))
        for property in margins { XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, .float(3)) }
        for property in weights + paddings { XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, .float(1)) }
        let stepper = try control(EVStyleStepper.self, "Adjust padding top", editor)
        stepper.doubleValue = 6
        XCTAssertTrue(stepper.sendAction(try XCTUnwrap(stepper.action), to: stepper.target))
        for property in paddings { XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, .float(6)) }
        let toggle = try control(NSButton.self, "Override padding right", editor)
        toggle.performClick(nil)
        for property in paddings {
            XCTAssertNil(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared)
            XCTAssertEqual(try control(NSButton.self, "Override \(property.displayName.lowercased())", editor).state, .off)
        }
        toggle.performClick(nil)
        for property in paddings { XCTAssertEqual(try control(NSButton.self, "Override \(property.displayName.lowercased())", editor).state, .on) }
        let colorToggle = try control(NSButton.self, "Override border bottom color", editor)
        if colorToggle.state == .off { colorToggle.performClick(nil) }
        let well = try control(NSColorWell.self, "Border bottom color", editor)
        well.color = NSColor(srgbRed: 0.3, green: 0.6, blue: 0.9, alpha: 0)
        XCTAssertTrue(well.sendAction(try XCTUnwrap(well.action), to: well.target))
        for property in colors {
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared,
                .color(EVStyleColor(red: 0.3, green: 0.6, blue: 0.9, alpha: 0)))
        }
        colorToggle.performClick(nil)
        for property in colors { XCTAssertNil(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared) }
    }

    func testUnspecifiedBorderColorsFollowTextInControlsAndPreviewUntilOverridden() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.block)
        for property in colors { XCTAssertTrue(editor.useInheritedForTesting(property)) }
        for property in weights { XCTAssertTrue(editor.setPropertyForTesting(property, value: .float(2))) }
        try control(NSButton.self, "Link block sides", editor).performClick(nil)
        for foreground in [EVStyleColor(red: 0.2, green: 0.4, blue: 0.7, alpha: 1), EVStyleColor(red: 0.7, green: 0.3, blue: 0.2, alpha: 1)] {
            XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(foreground)))
            for property in colors {
                XCTAssertNil(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared)
                XCTAssertEqual(try control(NSColorWell.self, property.displayName, editor).color, foreground.appKitColor)
            }
            let preview = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
            XCTAssertNil(preview.blockPreviewError)
            let borders = preview.boxes.filter { !$0.isBackground }
            XCTAssertFalse(borders.isEmpty)
            XCTAssertTrue(borders.allSatisfy { $0.color == foreground })
        }
        XCTAssertTrue(editor.useInheritedForTesting(.characterForeground))
        let themeColor = editor.themeStore.theme.foreground
        let expected = EVStyleColor(red: Float(themeColor.red), green: Float(themeColor.green), blue: Float(themeColor.blue), alpha: Float(themeColor.alpha))
        for property in colors { XCTAssertEqual(try control(NSColorWell.self, property.displayName, editor).color, expected.appKitColor) }
        let themePreview = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
        XCTAssertTrue(themePreview.boxes.filter { !$0.isBackground }.allSatisfy { $0.color == expected })
        let text = EVStyleValue.color(expected)
        try control(NSButton.self, "Override border top color", editor).performClick(nil)
        for property in colors { XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote)?.properties[property]?.declared, text) }
    }

    func testThemeUndoRetainsLinkSettingAndRestoresTheWholeRow() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-linked-theme-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let session = try EVThemeStyleSession(configuration: configuration, format: .markdown)
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(settingsSession: session)
        editor.selectStyle(quote)
        editor.selectTab(.block)
        for property in margins { XCTAssertTrue(editor.setPropertyForTesting(property, value: .float(1))) }
        let lock = try control(NSButton.self, "Link block sides", editor)
        lock.performClick(nil)
        let before = try session.snapshot().definition(for: quote)
        let bottom = try control(NSTextField.self, "Margin bottom", editor)
        bottom.stringValue = "3"
        bottom.delegate?.controlTextDidChange?(Notification(name: NSControl.textDidChangeNotification, object: bottom))
        let changed = try session.snapshot().definition(for: quote)
        for property in margins { XCTAssertEqual(changed?.properties[property]?.declared, .float(3)) }
        session.undoManager.undo()
        XCTAssertEqual(try session.snapshot().definition(for: quote), before)
        XCTAssertEqual(lock.state, .on, "Theme history rebuilds its specimen without changing the linking preference")
        session.undoManager.redo()
        XCTAssertEqual(try session.snapshot().definition(for: quote), changed)
        XCTAssertEqual(lock.state, .on)
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
