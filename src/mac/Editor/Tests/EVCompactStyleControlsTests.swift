import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVCompactStyleControlsTests: XCTestCase {

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

    private func makeEditor(theme: EVTheme = .paper) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, EVThemeStore) {
        let suite = "viem-style-theme-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
        let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
        addTeardownBlock { defaults.removePersistentDomain(forName: suite) }
        let themeStore = EVThemeStore(configuration: configuration)
        themeStore.update(theme)
        let heading = EVStyleTestFixtures.block("Heading1")
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontFace, .string("")),
            (.baseParagraph, .characterFontAxes, .string(EVFontVariations.encode([:]))),
            (.baseParagraph, .characterWeight, .unsigned(400)),
            (.baseParagraph, .paragraphFirstLineIndent, .float(0)),
            (.baseParagraph, .paragraphAlignment, .paragraphAlignment(UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START))),
            (.baseParagraph, .paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 1.1))),
            (heading, .characterSize, .float(24)),
            (heading, .paragraphFirstLineIndent, nil),
            (heading, .characterBold, nil),
            (heading, .characterUnderline, nil),
            (EVStyleTestFixtures.character("Code"), .characterSize, nil),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.readUntitled(source: Data("Text".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = themeStore
        editor.retarget(document: surface, styleKey: .baseParagraph)
        return (backend, surface, editor, themeStore)
    }

    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        return try XCTUnwrap(descendants(root).first { $0 is T && $0.accessibilityLabel() == label } as? T, "Missing control \(label)")
    }

    func testFamilySelectionKeepsLightFaceAndFallbacksThroughDuplicateNativeCallbacksAsOneUndo() throws {
        let (backend, surface, editor, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let initial = try XCTUnwrap(EVFontCatalog.faces(for: "SF Pro").first { $0.styleName == "Light" })
        let expected = try XCTUnwrap(EVFontCatalog.faces(for: "Helvetica Neue").first { $0.styleName == "Light" })
        let tail = ["Georgia", "Apple Color Emoji", "Menlo"]
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList([initial.familyName] + tail)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFace, value: .string(initial.styleName)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontAxes, value: .string(EVFontVariations.encode([:]))))
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
        XCTAssertEqual(definition.properties[.characterFontFamilies]?.declared, .stringList([expected.familyName] + tail))
        XCTAssertEqual(definition.properties[.characterFontFace]?.declared, .string(expected.styleName))
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

}
