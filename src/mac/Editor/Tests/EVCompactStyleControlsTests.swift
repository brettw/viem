import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVCompactStyleControlsTests: XCTestCase {
    func testDefaultParagraphClearsCharacterParentAndUnchangedMenusAreRetained() throws {
        let (_, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let code = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code"))
        let parentKey = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Comment"))
        editor.selectStyle(code)
        XCTAssertTrue(editor.setParentForTesting(parentKey))
        let parent = try control(NSPopUpButton.self, label: "Based on style", in: editor.view)
        parent.selectItem(withTitle: "Default Paragraph")
        XCTAssertTrue(parent.sendAction(try XCTUnwrap(parent.action), to: parent.target))
        XCTAssertEqual(editor.inspection.parentValue, "Default Paragraph")
        let style = try control(NSPopUpButton.self, label: "Style", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let following = try control(NSPopUpButton.self, label: "Following paragraph style", in: editor.view)
        let entries = [style, parent, face, following].map { $0.itemArray }
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(19)))
        for (popup, old) in zip([style, parent, face, following], entries) {
            XCTAssertEqual(popup.itemArray.count, old.count)
            for (a, b) in zip(popup.itemArray, old) { XCTAssertTrue(a === b) }
        }
    }

    func testFontSizeUnitConversionTracksBasedOnParagraphAndKeepsDeclaration() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(12)))
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(24)))
        let unit = try control(NSPopUpButton.self, label: "Font size unit", in: editor.view)
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        let stepper = try control(EVStyleStepper.self, label: "Adjust size", in: editor.view)
        XCTAssertEqual(unit.itemTitles, ["pt", "%"])
        XCTAssertEqual(unit.titleOfSelectedItem, "pt")
        unit.selectItem(withTitle: "%")
        XCTAssertTrue(unit.sendAction(try XCTUnwrap(unit.action), to: unit.target))
        XCTAssertEqual(size.stringValue, "200")
        XCTAssertEqual(stepper.minValue, 10)
        XCTAssertEqual(stepper.maxValue, 1000)
        XCTAssertEqual(stepper.increment, 1)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared, .percentage(200))
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(24))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(unit.titleOfSelectedItem, "pt")
        XCTAssertEqual(size.stringValue, "24")
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(unit.titleOfSelectedItem, "%")
        XCTAssertEqual(size.stringValue, "200")

        editor.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertEqual(unit.titleOfSelectedItem, "pt")
        XCTAssertFalse(try XCTUnwrap(unit.item(withTitle: "%")).isEnabled)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(14)))
        editor.selectStyle(heading)
        XCTAssertEqual(unit.titleOfSelectedItem, "%")
        XCTAssertEqual(size.stringValue, "200")
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(28))

        let child = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading2"))
        editor.selectStyle(child)
        XCTAssertTrue(editor.setParentForTesting(heading))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(21)))
        unit.selectItem(withTitle: "%")
        XCTAssertTrue(unit.sendAction(try XCTUnwrap(unit.action), to: unit.target))
        XCTAssertEqual(size.stringValue, "75", "A paragraph percentage uses its actual based-on style")
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterSize], .float(21))
        unit.selectItem(withTitle: "pt")
        XCTAssertTrue(unit.sendAction(try XCTUnwrap(unit.action), to: unit.target))
        XCTAssertEqual(size.stringValue, "21")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: child)?.properties[.characterSize]?.declared, .float(21))

        editor.selectStyle(EVStyleKey.baseParagraph)
        let before = try backend.styleSheetSnapshot()
        unit.selectItem(withTitle: "%")
        XCTAssertTrue(unit.sendAction(try XCTUnwrap(unit.action), to: unit.target))
        XCTAssertEqual(unit.titleOfSelectedItem, "pt")
        XCTAssertEqual(try backend.styleSheetSnapshot(), before, "Base Paragraph cannot declare a relative size even through an action sent directly")
    }

    func testFontSizePercentageDraftsRequireIntegersWithinBoundsAndStepperUsesPercent() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .percentage(200)))
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        let stepper = try control(EVStyleStepper.self, label: "Adjust size", in: editor.view)
        let delegate = try XCTUnwrap(size.delegate as? EVCompactStyleControls)
        let before = try backend.styleSheetSnapshot()
        for invalid in ["", "9", "1001", "99.5", "NaN", "-10", "1e2"] {
            size.stringValue = invalid
            delegate.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: size))
            XCTAssertTrue(editor.inspection.hasInvalidDraft, invalid)
            XCTAssertFalse(stepper.isEnabled, invalid)
            XCTAssertEqual(try backend.styleSheetSnapshot(), before, invalid)
        }
        for valid in [10, 1000, 90] {
            size.stringValue = String(valid)
            delegate.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: size))
            XCTAssertFalse(editor.inspection.hasInvalidDraft)
            XCTAssertTrue(stepper.isEnabled)
            XCTAssertEqual(stepper.doubleValue, Double(valid))
            XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared, .percentage(UInt32(valid)))
        }
        stepper.doubleValue += stepper.increment
        XCTAssertTrue(stepper.sendAction(try XCTUnwrap(stepper.action), to: stepper.target))
        XCTAssertEqual(size.stringValue, "91")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared, .percentage(91))
    }

    func testFontSizeUnitConversionRoundsAndClampsOnlyThePercentageDeclaration() throws {
        let (_, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(12)))
        editor.selectStyle(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
        let unit = try control(NSPopUpButton.self, label: "Font size unit", in: editor.view)
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        let cases: [(Float, UInt32, Float)] = [(10, 83, 9.96), (0.1, 10, 1.2), (200, 1000, 120)]
        for (points, percent, resolved) in cases {
            XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(points)))
            unit.selectItem(withTitle: "%")
            XCTAssertTrue(unit.sendAction(try XCTUnwrap(unit.action), to: unit.target))
            XCTAssertEqual(size.stringValue, String(percent))
            guard case let .float(actual)? = editor.inspection.preview.effectiveValues[.characterSize] else { return XCTFail("Expected points") }
            XCTAssertEqual(actual, resolved, accuracy: 0.0001)
        }
    }

    func testFirstClickOnInheritedFontSizeUnitActivatesPercentageAsOneUndoGesture() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.useInheritedForTesting(.characterSize))
        let unit = try control(NSPopUpButton.self, label: "Font size unit", in: editor.view)
        let checkbox = try control(NSButton.self, label: "Override font size", in: editor.view)
        XCTAssertFalse(unit.isEnabled)
        try inheritedWrapper(for: unit).activateAndPerform(unit) {
            unit.selectItem(withTitle: "%")
            XCTAssertTrue(unit.sendAction(unit.action, to: unit.target))
        }
        XCTAssertTrue(unit.isEnabled)
        XCTAssertEqual(checkbox.state, .on)
        XCTAssertEqual(unit.titleOfSelectedItem, "%")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared, .percentage(100))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared)
        XCTAssertFalse(unit.isEnabled)
        XCTAssertEqual(checkbox.state, .off)
    }

    func testInheritedFieldsAreEmptyUntilOverrideAndUncheckingRestoresInheritance() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.useInheritedForTesting(.characterSize))
        XCTAssertTrue(editor.useInheritedForTesting(.characterFontFamilies))
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let override = try control(NSButton.self, label: "Override font size", in: editor.view)
        XCTAssertEqual(size.stringValue, "")
        XCTAssertEqual(family.stringValue, "")
        XCTAssertNil(face.titleOfSelectedItem)
        XCTAssertFalse(size.isEnabled)
        XCTAssertFalse(family.isEnabled)
        XCTAssertFalse(face.isEnabled)
        XCTAssertEqual(override.title, "")
        XCTAssertEqual(override.toolTip, "Override inherited")
        XCTAssertEqual(override.state, .off)
        override.performClick(nil)
        XCTAssertEqual(override.state, .on)
        XCTAssertTrue(size.isEnabled)
        XCTAssertEqual(size.floatValue, 14)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared, .float(14))
        override.performClick(nil)
        XCTAssertEqual(override.state, .off)
        XCTAssertEqual(size.stringValue, "")
        XCTAssertFalse(size.isEnabled)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterSize]?.declared)
    }

    func testFontFaceUsesOneInheritanceCheckbox() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.useInheritedForTesting(.characterFontFamilies))
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let checkbox = try control(NSButton.self, label: "Override font families", in: editor.view)
        XCTAssertFalse(family.isEnabled); XCTAssertFalse(face.isEnabled)
        checkbox.performClick(nil)
        XCTAssertTrue(family.isEnabled); XCTAssertTrue(face.isEnabled)
        checkbox.performClick(nil)
        let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(for: heading))
        XCTAssertNil(definition.properties[.characterFontFamilies]?.declared)
        XCTAssertNil(definition.properties[.characterWeight]?.declared)
        XCTAssertNil(definition.properties[.characterFontAxes]?.declared)
        XCTAssertFalse(family.isEnabled); XCTAssertFalse(face.isEnabled)
    }

    func testFirstClickOnInheritedBoldTogglesBoldAndUndoesAsOneAction() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        let bold = try control(NSButton.self, label: "Bold", in: editor.view)
        let override = try control(NSButton.self, label: "Override bold", in: editor.view)
        XCTAssertFalse(bold.isEnabled)
        XCTAssertEqual(bold.state, .off)
        try performFirstClick(bold)
        XCTAssertTrue(bold.isEnabled)
        XCTAssertEqual(bold.state, .on)
        XCTAssertEqual(override.state, .on)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterBold]?.declared, .boolean(true))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterBold]?.declared)
        XCTAssertFalse(bold.isEnabled)
        XCTAssertFalse(surface.canUndo, "Activation and the forwarded click are a single gesture")
    }

    func testFirstClickOnInheritedStepperIncrementsAndUndoesAsOneAction() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        let stepper = try control(EVStyleStepper.self, label: "Adjust first line", in: editor.view)
        XCTAssertFalse(stepper.isEnabled)
        try inheritedWrapper(for: stepper).activateAndPerform(stepper) {
            stepper.performTrackingGesture {
                stepper.doubleValue += stepper.increment
                XCTAssertTrue(stepper.sendAction(stepper.action, to: stepper.target))
            }
        }
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.paragraphFirstLineIndent]?.declared, .float(1))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.paragraphFirstLineIndent]?.declared)
        XCTAssertFalse(surface.canUndo)
    }

    func testFirstMouseClickOnInheritedFontFaceOpensItsNativePopup() throws {
        let (_, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectStyle(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
        XCTAssertTrue(editor.useInheritedForTesting(.characterFontFamilies))
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let override = try control(NSButton.self, label: "Override font families", in: editor.view)
        XCTAssertFalse(face.isEnabled)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 650),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        editor.view.layoutSubtreeIfNeeded()
        var beganTracking = false
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification,
            object: face.menu, queue: .main) { _ in
                beganTracking = true
                DispatchQueue.main.async { face.menu?.cancelTracking() }
            }
        defer { NotificationCenter.default.removeObserver(observer) }
        try dispatchNativeFirstClick(face)
        XCTAssertTrue(beganTracking, "The first click must open the native face popup")
        XCTAssertTrue(face.isEnabled)
        XCTAssertEqual(override.state, .on)
    }

    func testFirstMouseClickOnInheritedSizeFocusesItsNativeTextEditor() throws {
        let (_, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        XCTAssertFalse(size.isEnabled)
        XCTAssertEqual(size.stringValue, "")
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 650),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        editor.view.layoutSubtreeIfNeeded()
        try dispatchNativeFirstClick(size)
        XCTAssertTrue(size.isEnabled)
        let fieldEditor = try XCTUnwrap(size.currentEditor() as? NSTextView,
            "The first click must enter the native text field")
        XCTAssertEqual(size.floatValue, 14)
        XCTAssertTrue(editor.hasActiveStyleEditGroupForTesting)
        for text in ["18", "19"] {
            fieldEditor.insertText(text, replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        }
        XCTAssertEqual(size.floatValue, 19)
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(size.stringValue, "")
        XCTAssertFalse(size.isEnabled)
        XCTAssertFalse(surface.canUndo, "The initial activation and subsequent field typing share one undo gesture")
    }

    private func dispatchNativeFirstClick(_ control: NSControl) throws {
        let window = try XCTUnwrap(control.window)
        let wrapper = try inheritedWrapper(for: control)
        let location = control.convert(NSPoint(x: control.bounds.midX, y: control.bounds.midY), to: nil)
        let parent = try XCTUnwrap(wrapper.superview)
        let hit = try XCTUnwrap(wrapper.hitTest(parent.convert(location, from: nil)))
        XCTAssertTrue(hit === wrapper)
        func event(_ type: NSEvent.EventType) throws -> NSEvent {
            try XCTUnwrap(NSEvent.mouseEvent(with: type, location: location, modifierFlags: [],
                timestamp: 0, windowNumber: window.windowNumber, context: nil,
                eventNumber: 1, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0))
        }
        NSApplication.shared.postEvent(try event(.leftMouseUp), atStart: true)
        hit.mouseDown(with: try event(.leftMouseDown))
    }

    func testInheritedNativeControlReceivesTheOriginalMouseDownOnlyAfterActivation() throws {
        final class ClickRecorder: NSControl {
            var recordedEvent: NSEvent?
            override func mouseDown(with event: NSEvent) {
                XCTAssertTrue(isEnabled)
                recordedEvent = event
            }
        }
        let control = ClickRecorder(frame: NSRect(x: 0, y: 0, width: 100, height: 30))
        control.widthAnchor.constraint(equalToConstant: 100).isActive = true
        control.heightAnchor.constraint(equalToConstant: 30).isActive = true
        let wrapper = EVInheritedStyleControl(views: [control])
        wrapper.frame = NSRect(x: 10, y: 10, width: 100, height: 30)
        let host = NSView(frame: NSRect(x: 0, y: 0, width: 180, height: 100))
        host.addSubview(wrapper)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        control.isEnabled = false
        var activationCount = 0
        wrapper.canActivate = { !control.isEnabled }
        wrapper.activate = { activationCount += 1; control.isEnabled = true }
        let local = NSPoint(x: control.bounds.midX, y: control.bounds.midY)
        let location = control.convert(local, to: nil)
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location,
            modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber,
            context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        XCTAssertTrue(wrapper.hitTest(host.convert(location, from: nil)) === wrapper)
        XCTAssertEqual(activationCount, 0, "Hit testing and hovering do not declare values")
        wrapper.mouseDown(with: event)
        XCTAssertEqual(activationCount, 1)
        XCTAssertTrue(control.recordedEvent === event, "The font popup, toggle, or text field must receive the same click")
        control.isEnabled = false
        wrapper.canActivate = { false }
        wrapper.mouseDown(with: event)
        XCTAssertEqual(activationCount, 1, "Read-only controls cannot activate")
    }

    func testUnderlineButtonHasVisibleUnderlineAndNativeAction() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let button = try control(NSButton.self, label: "Underline", in: editor.view)
        XCTAssertEqual(button.attributedTitle.string, "U")
        XCTAssertEqual(button.attributedTitle.attribute(.underlineStyle, at: 0, effectiveRange: nil) as? Int, NSUnderlineStyle.single.rawValue)
        button.performClick(nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterUnderline]?.declared, .boolean(true))
    }
    private func makeEditor(theme: EVTheme = .paper) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, EVThemeStore) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("Text".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let suite = "viem-style-theme-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
        let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
        addTeardownBlock { defaults.removePersistentDomain(forName: suite) }
        let themeStore = EVThemeStore(configuration: configuration)
        themeStore.update(theme)
        let editor = EVStyleEditorViewController()
        editor.themeStore = themeStore
        editor.retarget(document: surface, styleKey: .baseParagraph)
        return (backend, surface, editor, themeStore)
    }

    private func inheritedWrapper(for control: NSView) throws -> EVInheritedStyleControl {
        var ancestor = control.superview
        while let view = ancestor {
            if let wrapper = view as? EVInheritedStyleControl { return wrapper }
            ancestor = view.superview
        }
        return try XCTUnwrap(nil as EVInheritedStyleControl?, "Missing inherited-control activation wrapper")
    }

    private func performFirstClick(_ button: NSButton) throws {
        if button.isEnabled { button.performClick(nil) }
        else { try inheritedWrapper(for: button).activateAndPerform(button) { button.performClick(nil) } }
    }

    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        return try XCTUnwrap(descendants(root).first { $0 is T && $0.accessibilityLabel() == label } as? T, "Missing control \(label)")
    }

    func testParagraphControlsUseCoreEnumValuesForDisplayAndNativeActions() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        editor.selectTab(.paragraph)
        let alignment = try control(NSSegmentedControl.self, label: "Paragraph alignment", in: editor.view)
        let lineKind = try control(NSPopUpButton.self, label: "Line spacing kind", in: editor.view)
        let lineValue = try control(NSTextField.self, label: "Line spacing value", in: editor.view)
        let lineStepper = try control(EVStyleStepper.self, label: "Adjust line spacing value", in: editor.view)
        let lineUnit = try control(NSTextField.self, label: "Line spacing unit", in: editor.view)
        XCTAssertEqual(alignment.selectedSegment, 0, "Start is core enum1 and UI segment0")
        XCTAssertEqual(lineKind.titleOfSelectedItem, "Normal")
        XCTAssertEqual(lineValue.stringValue, "1")
        XCTAssertTrue(lineValue.isEnabled)
        XCTAssertTrue(lineStepper.isEnabled)
        XCTAssertEqual(lineStepper.increment, 0.1)
        XCTAssertEqual(lineUnit.stringValue, "×")
        for (segment, expected) in [(1, VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER), (2, VIEM_STYLE_PARAGRAPH_ALIGNMENT_END), (0, VIEM_STYLE_PARAGRAPH_ALIGNMENT_START)] {
            alignment.selectedSegment = segment
            XCTAssertTrue(alignment.sendAction(try XCTUnwrap(alignment.action), to: alignment.target))
            let value = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphAlignment]?.declared
            XCTAssertEqual(value, .paragraphAlignment(UInt32(expected)))
            XCTAssertEqual(alignment.selectedSegment, segment)
            XCTAssertEqual(editor.inspection.diagnostic, "")
        }
        for (title, kind) in [("Multiple", VIEM_STYLE_LINE_SPACING_MULTIPLIER), ("At least", VIEM_STYLE_LINE_SPACING_AT_LEAST), ("Exactly", VIEM_STYLE_LINE_SPACING_EXACT), ("Normal", VIEM_STYLE_LINE_SPACING_NORMAL)] {
            lineKind.selectItem(withTitle: title)
            XCTAssertTrue(lineKind.sendAction(try XCTUnwrap(lineKind.action), to: lineKind.target))
            let value = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared
            guard case let .lineSpacing(spacing)? = value else { return XCTFail("Native line spacing choice did not write a declaration") }
            XCTAssertEqual(spacing.kind, UInt32(kind))
            if title == "Normal" {
                XCTAssertEqual(lineValue.stringValue, "1")
                XCTAssertEqual(lineUnit.stringValue, "×")
            } else {
                XCTAssertGreaterThan(spacing.value, 0)
                XCTAssertEqual(lineUnit.stringValue, title == "Multiple" ? "×" : "pt")
            }
            XCTAssertTrue(lineValue.isEnabled)
            XCTAssertEqual(lineKind.titleOfSelectedItem, title)
            XCTAssertEqual(editor.inspection.diagnostic, "")
        }
    }

    func testDefaultThemeSwatchAndPreviewUpdateWithoutMutatingSourceOrStyle() throws {
        let (backend, surface, editor, store) = try makeEditor(theme: .midnight)
        defer { withExtendedLifetime(surface) {} }
        let swatch = try control(NSColorWell.self, label: "Text color", in: editor.view)
        let before = try backend.styleSheetSnapshot()
        let bytes = try backend.serializedSource(typeName: EVDocument.markdownType)
        func styleColor(_ color: EVThemeColor) -> EVStyleColor { EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)) }
        XCTAssertEqual(EVThemeColor(swatch.color), EVTheme.midnight.foreground)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterForeground], .color(styleColor(EVTheme.midnight.foreground)))
        XCTAssertEqual(editor.inspection.preview.canvasBackground, styleColor(EVTheme.midnight.background))
        store.update(.paper)
        XCTAssertEqual(EVThemeColor(swatch.color), EVTheme.paper.foreground)
        XCTAssertEqual(editor.inspection.preview.canvasBackground, styleColor(EVTheme.paper.background))
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), bytes)
    }

    func testExplicitAndInheritedBlackRemainBlackUntilNativeDefaultAction() throws {
        let (backend, surface, editor, store) = try makeEditor(theme: .midnight)
        defer { withExtendedLifetime(surface) {} }
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        let swatch = try control(NSColorWell.self, label: "Text color", in: editor.view)
        swatch.color = .black
        XCTAssertTrue(swatch.sendAction(try XCTUnwrap(swatch.action), to: swatch.target))
        let black = EVStyleColor(red: 0, green: 0, blue: 0, alpha: 1)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterForeground], .color(black))
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterForeground]?.declared, .color(black))
        store.update(.paper); store.update(.midnight)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterForeground], .color(black))
        let reset = try control(NSButton.self, label: "Override foreground", in: editor.view)
        reset.performClick(nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading)?.properties[.characterForeground]?.declared)
        XCTAssertEqual(EVThemeColor(swatch.color), EVTheme.midnight.foreground)
        editor.selectStyle(EVStyleKey.baseParagraph)
        swatch.color = .black
        XCTAssertTrue(swatch.sendAction(try XCTUnwrap(swatch.action), to: swatch.target))
        editor.selectStyle(heading)
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterForeground], .color(black), "An explicit declaration inherited from Base Paragraph is preserved")
    }

    func testNativeLightFaceThenBoldCommitsSourceBackedStyle() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        XCTAssertEqual(family.numberOfVisibleItems, 20)
        family.stringValue = "SF Pro"
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let light = try XCTUnwrap(face.itemArray.first { $0.title == "Light" })
        face.select(light)
        XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
        let bold = try control(NSButton.self, label: "Bold", in: editor.view)
        try performFirstClick(bold)
        XCTAssertEqual(editor.inspection.diagnostic, "")
        XCTAssertEqual(bold.state, .on)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterBold]?.declared, .boolean(true))
        XCTAssertEqual(family.stringValue, "SF Pro")
    }

    func testPrimaryFamilyAndFaceKeepOrderedFallbackTail() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let tail = ["Georgia", "Apple Color Emoji", "Menlo"]
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList(["Helvetica"] + tail)))
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        family.stringValue = "SF Pro"
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        face.select(try XCTUnwrap(face.itemArray.first { $0.title == "Light" }))
        XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
        guard case let .stringList(request)? = try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared else { return XCTFail("Missing font request") }
        XCTAssertEqual(Array(request.dropFirst()), tail)
        XCTAssertTrue(request[0].contains("Light"))

    }

    func testFamilySelectionKeepsLightFaceAndFallbacksThroughDuplicateNativeCallbacksAsOneUndo() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let initial = try XCTUnwrap(EVFontCatalog.faces(for: "SF Pro").first { $0.styleName == "Light" })
        let expected = try XCTUnwrap(EVFontCatalog.faces(for: "Helvetica Neue").first { $0.styleName == "Light" })
        let tail = ["Georgia", "Apple Color Emoji", "Menlo"]
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList([initial.postScriptName] + tail)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(UInt32(initial.weight))))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(initial.italic ? 1 : 0)))
        let source = try backend.serializedSource(typeName: EVDocument.markdownType)
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let owner = try XCTUnwrap(family.target as? EVCompactStyleControls)
        let publish = try XCTUnwrap(owner.onMutations)
        var batches = 0
        owner.onMutations = { [weak owner] mutations in
            batches += 1
            // Programmatic control refresh must not reenter the publication.
            owner?.comboBoxSelectionDidChange(Notification(name: NSComboBox.selectionDidChangeNotification, object: family))
            publish(mutations)
        }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 780, height: 770), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(family))
        let fieldEditor = try XCTUnwrap(family.currentEditor() as? NSTextView)
        XCTAssertEqual(fieldEditor.string, "SF Pro")
        family.selectItem(withObjectValue: "Helvetica Neue")
        owner.comboBoxSelectionDidChange(Notification(name: NSComboBox.selectionDidChangeNotification, object: family))
        XCTAssertEqual(fieldEditor.string, "Helvetica Neue", "Selection must replace the active field editor's old family too")
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(batches, 1, "Selection, action, and editing-end callbacks represent one family choice")
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        XCTAssertEqual(family.stringValue, "Helvetica Neue")
        XCTAssertEqual(face.titleOfSelectedItem, "Light")
        let definition = try XCTUnwrap(try backend.styleSheetSnapshot().definition(for: .baseParagraph))
        XCTAssertEqual(definition.properties[.characterFontFamilies]?.declared, .stringList([expected.postScriptName] + tail))
        XCTAssertEqual(definition.properties[.characterWeight]?.declared, .unsigned(UInt32(expected.weight)))
        XCTAssertEqual(definition.properties[.characterSlant]?.declared, .fontSlant(initial.italic ? 1 : 0), "Choosing a base face preserves semantic Italic")
        let changed = try backend.serializedSource(typeName: EVDocument.markdownType)
        XCTAssertEqual(changed, source)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source, "One undo restores family, weight, and slant together")
        XCTAssertEqual(family.stringValue, "SF Pro")
        XCTAssertEqual(face.titleOfSelectedItem, "Light")
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), changed)
        XCTAssertEqual(family.stringValue, "Helvetica Neue")
        XCTAssertEqual(face.titleOfSelectedItem, "Light")
    }

    func testTypedFamilyUsesRegularWhenPriorNamedFaceDoesNotExist() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let light = try XCTUnwrap(EVFontCatalog.faces(for: "Helvetica Neue").first { $0.styleName == "Light" })
        let georgia = EVFontCatalog.faces(for: "Georgia")
        XCTAssertFalse(georgia.contains { $0.styleName == "Light" })
        let regular = try XCTUnwrap(georgia.first { $0.styleName == "Regular" && !$0.italic })
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList([light.postScriptName, "Menlo"])))
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(UInt32(light.weight))))
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 780, height: 770), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(family))
        let fieldEditor = try XCTUnwrap(family.currentEditor() as? NSTextView)
        fieldEditor.insertText("Georgia", replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(family.stringValue, "Georgia")
        XCTAssertEqual(face.titleOfSelectedItem, "Regular")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList([regular.postScriptName, "Menlo"]))
        XCTAssertEqual(editor.inspection.preview.effectiveValues[.characterWeight], .unsigned(UInt32(regular.weight)))
    }

    func testUnmatchedFaceAndUnavailableFamilyDoNotSelectAnUnrelatedFirstFace() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList(["Helvetica", "Menlo"])))
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(700)))
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        XCTAssertEqual(face.titleOfSelectedItem, "Bold", "Helvetica is also its regular PostScript name, but the declared base weight selects Bold")
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList(["Helvetica Neue", "Menlo"])))
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(333)))
        XCTAssertGreaterThan(face.numberOfItems, 0)
        XCTAssertEqual(face.indexOfSelectedItem, -1, "No exact face matches weight 333; displaying the first Light face would misstate the request")
        let unavailable = "Viem Missing Font \(UUID().uuidString)"
        family.stringValue = unavailable
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        XCTAssertEqual(family.stringValue, unavailable)
        XCTAssertEqual(face.numberOfItems, 0)
        XCTAssertEqual(face.indexOfSelectedItem, -1)
        XCTAssertFalse(face.isEnabled)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList([unavailable, "Menlo"]))
    }

    func testRejectedFamilyMutationRestoresCommittedComboAndActiveFieldEditor() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let light = try XCTUnwrap(EVFontCatalog.faces(for: "Helvetica Neue").first { $0.styleName == "Light" })
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList([light.postScriptName])))
        XCTAssertTrue(editor.setPropertyForTesting(.characterWeight, value: .unsigned(UInt32(light.weight))))
        let before = try backend.styleSheetSnapshot()
        let family = try control(NSComboBox.self, label: "Font family", in: editor.view)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let owner = try XCTUnwrap(family.target as? EVCompactStyleControls)
        var attempts = 0
        owner.onMutations = { _ in attempts += 1 } // A rejected publication provides no new committed definition.
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 780, height: 770), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(family))
        let fieldEditor = try XCTUnwrap(family.currentEditor() as? NSTextView)
        family.selectItem(withObjectValue: "Georgia")
        owner.comboBoxSelectionDidChange(Notification(name: NSComboBox.selectionDidChangeNotification, object: family))
        XCTAssertEqual(attempts, 1)
        XCTAssertEqual(family.stringValue, "Helvetica Neue")
        XCTAssertEqual(family.objectValueOfSelectedItem as? String, "Helvetica Neue")
        XCTAssertEqual(fieldEditor.string, "Helvetica Neue")
        XCTAssertEqual(face.titleOfSelectedItem, "Light")
        XCTAssertTrue(family.sendAction(try XCTUnwrap(family.action), to: family.target))
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(attempts, 1, "Later callbacks cannot retry a rejected draft after restoring committed text")
        XCTAssertEqual(try backend.styleSheetSnapshot(), before)
    }

    func testNativeFallbackPopoverOrdersAddsRemovesAppliesAndCancels() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let original = ["Helvetica", "Georgia", "Apple Color Emoji"]
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList(original)))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 770), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        window.displayIfNeeded()
        let button = try control(NSButton.self, label: "Fallback fonts", in: editor.view)
        let owner = try XCTUnwrap(button.target as? EVCompactStyleControls)
        button.performClick(nil)
        let popover = try XCTUnwrap(owner.fallbackPopover)
        let draft = try XCTUnwrap(popover.contentViewController as? EVFallbackFontsController)
        let table = try control(NSTableView.self, label: "Ordered fallback fonts", in: draft.view)
        table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        try control(NSButton.self, label: "Move fallback up", in: draft.view).performClick(nil)
        let entry = try control(NSComboBox.self, label: "Add fallback family", in: draft.view)
        XCTAssertEqual(entry.numberOfVisibleItems, 20)
        XCTAssertTrue(try XCTUnwrap(entry.window).makeFirstResponder(entry))
        let fieldEditor = try XCTUnwrap(entry.currentEditor() as? NSTextView)
        fieldEditor.insertText("Menlo", replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        XCTAssertEqual(entry.stringValue, "Menlo")
        try control(NSButton.self, label: "Add fallback font", in: draft.view).performClick(nil)
        table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        try control(NSButton.self, label: "Remove fallback font", in: draft.view).performClick(nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList(original), "The popover is a draft until Apply")
        try control(NSButton.self, label: "Apply fallback fonts", in: draft.view).performClick(nil)
        let expected = ["Helvetica", "Apple Color Emoji", "Menlo"]
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList(expected))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList(original))
        button.performClick(nil)
        let cancelled = try XCTUnwrap(owner.fallbackPopover?.contentViewController as? EVFallbackFontsController)
        let cancelledTable = try control(NSTableView.self, label: "Ordered fallback fonts", in: cancelled.view)
        cancelledTable.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        try control(NSButton.self, label: "Remove fallback font", in: cancelled.view).performClick(nil)
        try control(NSButton.self, label: "Cancel fallback changes", in: cancelled.view).performClick(nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterFontFamilies]?.declared, .stringList(original))
        withExtendedLifetime(window) {}
    }

    func testEveryNumericControlHasAnAdjacentNativeStepperAndInheritedValue() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let fields = ["Size", "Tracking", "Start indent", "End indent", "First line", "Margin top", "Margin bottom", "Line spacing value"]
        for title in fields {
            let field = try control(NSTextField.self, label: title, in: editor.view)
            let stepper = try control(EVStyleStepper.self, label: "Adjust \(title.lowercased())", in: editor.view)
            XCTAssertTrue(stepper.superview === field.superview)
            let siblings = try XCTUnwrap(field.superview as? NSStackView).arrangedSubviews
            XCTAssertEqual(try XCTUnwrap(siblings.firstIndex(of: stepper)), try XCTUnwrap(siblings.firstIndex(of: field)) + 1)
            XCTAssertEqual(stepper.isEnabled, field.isEnabled)
            if field.isEnabled { XCTAssertEqual(stepper.doubleValue, field.doubleValue, accuracy: 0.0001) }
        }
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(14))
        let size = try control(EVStyleStepper.self, label: "Adjust size", in: editor.view)
        size.doubleValue += size.increment
        XCTAssertTrue(size.sendAction(try XCTUnwrap(size.action), to: size.target))
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(15))
        let baseOverride = try control(NSButton.self, label: "Override font size", in: editor.view)
        XCTAssertFalse(baseOverride.isEnabled)
        baseOverride.performClick(nil)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.declared, .float(15))
        XCTAssertEqual(size.doubleValue, 15)
        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertFalse(try control(EVStyleStepper.self, label: "Adjust start indent", in: editor.view).isEnabled)
    }

    func testLineSpacingDraftPreservesIntermediateDecimalTextAndCaretUntilEditingEnds() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 780, height: 770),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        editor.selectTab(.paragraph)
        let kind = try control(NSPopUpButton.self, label: "Line spacing kind", in: editor.view)
        let lineValue = try control(NSTextField.self, label: "Line spacing value", in: editor.view)
        XCTAssertTrue(window.makeFirstResponder(lineValue))
        let fieldEditor = try XCTUnwrap(lineValue.currentEditor() as? NSTextView)

        fieldEditor.insertText("1.", replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        XCTAssertEqual(fieldEditor.string, "1.")
        XCTAssertEqual(fieldEditor.selectedRange(), NSRange(location: 2, length: 0))
        fieldEditor.insertText("2", replacementRange: fieldEditor.selectedRange())
        fieldEditor.insertText("0", replacementRange: fieldEditor.selectedRange())
        XCTAssertEqual(fieldEditor.string, "1.20", "Live rounding must not rewrite an in-progress native field-editor draft")
        XCTAssertEqual(fieldEditor.selectedRange(), NSRange(location: 4, length: 0), "Live commits must preserve the insertion caret")
        XCTAssertEqual(kind.titleOfSelectedItem, "Multiple")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared,
                       .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 1.2)))

        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(lineValue.stringValue, "1.2", "Ending the edit canonicalizes the displayed value")
    }

    func testNumericFieldDraftAndLineSpacingPrecisionAndKindTransitions() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 780, height: 770), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        let size = try control(NSTextField.self, label: "Size", in: editor.view)
        let sizeStepper = try control(EVStyleStepper.self, label: "Adjust size", in: editor.view)
        let before = try backend.serializedSource(typeName: EVDocument.markdownType)
        XCTAssertTrue(window.makeFirstResponder(size))
        let fieldEditor = try XCTUnwrap(size.currentEditor() as? NSTextView)
        fieldEditor.insertText("-", replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        XCTAssertTrue(editor.inspection.hasInvalidDraft)
        XCTAssertFalse(sizeStepper.isEnabled)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), before)
        fieldEditor.insertText("18.5", replacementRange: NSRange(location: 0, length: fieldEditor.string.utf16.count))
        XCTAssertFalse(editor.inspection.hasInvalidDraft)
        XCTAssertTrue(sizeStepper.isEnabled)
        XCTAssertEqual(sizeStepper.doubleValue, 18.5)
        window.makeFirstResponder(nil)
        editor.selectTab(.paragraph)
        let kind = try control(NSPopUpButton.self, label: "Line spacing kind", in: editor.view)
        let lineValue = try control(NSTextField.self, label: "Line spacing value", in: editor.view)
        let line = try control(EVStyleStepper.self, label: "Adjust line spacing value", in: editor.view)
        let unit = try control(NSTextField.self, label: "Line spacing unit", in: editor.view)
        let spaceBefore = try control(NSTextField.self, label: "Margin top", in: editor.view)
        let spaceAfter = try control(NSTextField.self, label: "Margin bottom", in: editor.view)
        XCTAssertEqual(spaceBefore.constraints.first { $0.firstAttribute == .width }?.constant, 45)
        XCTAssertEqual(spaceAfter.constraints.first { $0.firstAttribute == .width }?.constant, 45)
        XCTAssertEqual(kind.titleOfSelectedItem, "Normal")
        XCTAssertEqual(lineValue.stringValue, "1")
        XCTAssertTrue(line.isEnabled)
        XCTAssertEqual(line.increment, 0.1)
        XCTAssertEqual(unit.stringValue, "×")

        line.doubleValue -= line.increment
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(kind.titleOfSelectedItem, "Multiple")
        XCTAssertEqual(lineValue.stringValue, "0.9")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared,
                       .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 0.9)))
        line.doubleValue += line.increment
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(kind.titleOfSelectedItem, "Normal")
        XCTAssertEqual(lineValue.stringValue, "1")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared,
                       .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_NORMAL), value: 0)))

        kind.selectItem(withTitle: "Multiple")
        XCTAssertTrue(kind.sendAction(try XCTUnwrap(kind.action), to: kind.target))
        XCTAssertEqual(lineValue.stringValue, "1.1", "Explicitly choosing Multiple from Normal must not immediately normalize back to Normal")
        XCTAssertEqual(line.increment, 0.1)
        XCTAssertEqual(unit.stringValue, "×")

        let noisy: Float = 0.7999999523
        XCTAssertTrue(editor.setPropertyForTesting(.paragraphLineSpacing,
            value: .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: noisy))))
        let sourceWithNoisyValue = try backend.serializedSource(typeName: EVDocument.markdownType)
        XCTAssertEqual(lineValue.stringValue, "0.8")
        XCTAssertEqual(line.doubleValue, 0.8, accuracy: 0.0001)
        guard case let .lineSpacing(noisyDeclaration)? = try backend.styleSheetSnapshot()
            .definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared else {
            return XCTFail("Missing noisy multiplier declaration")
        }
        XCTAssertEqual(noisyDeclaration.value, noisy, "Rendering rounds only the display and must not rewrite source")
        editor.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), sourceWithNoisyValue)

        let noisyPoints: Float = 12.299999237
        XCTAssertTrue(editor.setPropertyForTesting(.paragraphLineSpacing,
            value: .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_EXACT), value: noisyPoints))))
        XCTAssertEqual(kind.titleOfSelectedItem, "Exactly")
        XCTAssertEqual(lineValue.stringValue, "12.3")
        XCTAssertEqual(unit.stringValue, "pt")
        guard case let .lineSpacing(noisyPointDeclaration)? = try backend.styleSheetSnapshot()
            .definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared else {
            return XCTFail("Missing noisy point-valued declaration")
        }
        XCTAssertEqual(noisyPointDeclaration.value, noisyPoints,
            "Point-valued line spacing also rounds only its presentation")

        kind.selectItem(withTitle: "At least")
        XCTAssertTrue(kind.sendAction(try XCTUnwrap(kind.action), to: kind.target))
        XCTAssertEqual(line.increment, 1)
        XCTAssertEqual(unit.stringValue, "pt")
        line.doubleValue = 12.0000001
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(lineValue.stringValue, "12")
        line.doubleValue = 12.299999523
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(lineValue.stringValue, "12.3")
        line.doubleValue += line.increment
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(lineValue.stringValue, "13.3")
        kind.selectItem(withTitle: "Exactly")
        XCTAssertTrue(kind.sendAction(try XCTUnwrap(kind.action), to: kind.target))
        XCTAssertEqual(line.increment, 1)
        XCTAssertEqual(unit.stringValue, "pt")
        kind.selectItem(withTitle: "At least")
        XCTAssertTrue(kind.sendAction(try XCTUnwrap(kind.action), to: kind.target))
        line.doubleValue = 0
        XCTAssertTrue(line.sendAction(try XCTUnwrap(line.action), to: line.target))
        XCTAssertEqual(editor.inspection.diagnostic, "")
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.paragraphLineSpacing]?.declared,
                       .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST), value: 0)))
    }

}
