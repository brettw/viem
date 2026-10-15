import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVLinkPopoverTests: XCTestCase {

    private func editor(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: 350)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 100, width: 850, height: 350),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, window)
    }

    func testCommandKOpensInsertionOrEditingWithoutChangingExistingLinks() throws {
        for (type, source, selectionLength, text, destination) in [
            (EVDocument.markdownType, "word", 0, "", ""),
            (EVDocument.markdownType, "word", 4, "word", ""),
        ] {
            let (backend, surface, window) = try editor(source, type: type)
            defer { surface.linkPopover.close(); window.close() }
            let formatted = try backend.formattedText() as NSString
            let location = destination.isEmpty ? 0 : formatted.range(of: "label").location + 1
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: location, length: selectionLength))
            let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.command],
                timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "k",
                charactersIgnoringModifiers: "k", isARepeat: false, keyCode: 40))
            XCTAssertTrue(surface.editorView.performKeyEquivalent(with: event), type)
            XCTAssertTrue(surface.linkPopover.isEditing, type)
            XCTAssertEqual(surface.linkPopover.textField.stringValue, text, type)
            XCTAssertEqual(surface.linkPopover.destinationField.stringValue, destination, type)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), type)
            surface.linkPopover.close(restoreFocus: true)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), type)
            XCTAssertFalse(surface.canUndo)
        }
    }

    private func trackHeadingPicker(_ combo: NSComboBox, keys: [(UInt16, String)]) throws {
        let window = try XCTUnwrap(combo.window)
        _ = try XCTUnwrap(window.screen, "Native dropdown tracking requires WindowServer access; run the test host outside the execution sandbox")
        window.contentView?.layoutSubtreeIfNeeded()
        XCTAssertTrue(window.makeFirstResponder(combo))
        let location = combo.convert(NSPoint(x: combo.bounds.maxX - 8, y: combo.bounds.midY), to: nil)
        for (keyCode, characters) in keys.reversed() {
            let key = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: location, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
            NSApplication.shared.postEvent(key, atStart: true)
        }
        let up = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseUp, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 0))
        NSApplication.shared.postEvent(up, atStart: true)
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        combo.mouseDown(with: down)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    }

    func testNativeHeadingPickerEscapeKeepsDraftAndSelectionKeepsFragmentAfterTracking() throws {
        let source = "plain\n\n# Destination"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 0))
        surface.linkPopover.openEditor(inserting: true)
        let popup = surface.linkPopover
        let combo = try XCTUnwrap(popup.destinationField as? NSComboBox)
        popup.textField.stringValue = "draft"
        combo.stringValue = "#typed"
        try trackHeadingPicker(combo, keys: [(53, "\u{1b}")])
        XCTAssertTrue(popup.isEditing, "Escape dismisses the native heading list while retaining the form")
        XCTAssertEqual(popup.textField.stringValue, "draft")
        XCTAssertEqual(combo.stringValue, "#typed")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        try trackHeadingPicker(combo, keys: [(125, "\u{f701}"), (36, "\r")])
        XCTAssertTrue(popup.isEditing)
        XCTAssertEqual(combo.stringValue, "#destination", "Native tracking and field synchronization must retain the fragment destination")
        XCTAssertTrue(popup.applyButton.isEnabled)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testToolbarUsesSelectedTextAndAppliesOneUndoableLinkInBothViews() throws {
        for type in [EVDocument.markdownType] {
            let (backend, surface, window) = try editor("before words after", type: type)
            defer { surface.linkPopover.close(); window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 5))
            let toolbar = surface.formattingToolbar
            toolbar.refresh()
            XCTAssertTrue(toolbar.insertLink.isEnabled)
            toolbar.insertLink.performClick(nil)
            let popup = surface.linkPopover
            XCTAssertTrue(popup.isEditing)
            XCTAssertEqual(popup.textField.stringValue, "words")
            XCTAssertEqual(popup.destinationField.stringValue, "")
            popup.destinationField.stringValue = "docs/next.md#intro"
            popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
            popup.applyButton.performClick(nil)
            XCTAssertNil(surface.commandOutput)
            XCTAssertFalse(popup.isEditing)
            let linked = try backend.serializedSource(typeName: type)
            XCTAssertTrue(String(decoding: linked, as: UTF8.self).contains("docs/next.md#intro"))
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data("before words after".utf8))
            surface.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), linked)
        }
    }

    func testCompactPopupFitsLabelAndRetainsMaximumWidth() throws {
        var widths: [CGFloat] = []
        for label in ["x", String(repeating: "wide label ", count: 30)] {
            let (_, surface, window) = try editor("[\(label)](https://example.com)", type: EVDocument.markdownType)
            defer { surface.linkPopover.close(); window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 0))
            surface.linkPopover.refresh()
            XCTAssertTrue(surface.linkPopover.isOpen)
            XCTAssertEqual(surface.linkPopover.destinationButton.title, label)
            widths.append(surface.linkPopover.popupFrame.width)
        }
        XCTAssertLessThan(widths[0], widths[1])
        XCTAssertLessThanOrEqual(widths[1], 374)
        // An empty-label link has no text; its boundary carries the popup.
        let (_, surface, window) = try editor("a[](https://example.com)b", type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 0))
        surface.linkPopover.refresh()
        XCTAssertTrue(surface.linkPopover.isOpen)
        XCTAssertEqual(surface.linkPopover.destinationButton.title, "empty")
        let font = try XCTUnwrap(surface.linkPopover.destinationButton.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
        XCTAssertTrue(NSFontManager.shared.traits(of: font).contains(.italicFontMask))
    }

    func testOpeningDestinationDismissesPopupBeforeRouting() throws {
        let (_, surface, window) = try editor("[label](https://example.com)", type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 0))
        let popup = surface.linkPopover
        popup.refresh()
        XCTAssertTrue(popup.isOpen)
        var opened: [URL] = []
        surface.editorView.openLinkURL = { url, completion in
            XCTAssertFalse(popup.isOpen, "Dismiss the old popup before a destination can change the active pane")
            surface.refreshPresentation()
            XCTAssertFalse(popup.isOpen, "An unchanged origin caret must not reopen the popup during navigation")
            opened.append(url)
            completion(nil)
        }
        popup.destinationButton.performClick(nil)
        XCTAssertEqual(opened.map(\.absoluteString), ["https://example.com"])
        XCTAssertFalse(popup.isOpen)
    }

    func testCancelAndStaleSelectionCannotApplyDraft() throws {
        let (backend, surface, window) = try editor("one two", type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        surface.linkPopover.openEditor()
        let popup = surface.linkPopover
        popup.textField.stringValue = "draft"
        popup.destinationField.stringValue = "https://example.com"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        popup.cancel()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("one two".utf8))
        surface.linkPopover.openEditor()
        popup.textField.stringValue = "draft"
        popup.destinationField.stringValue = "https://example.com"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 4, length: 0))
        XCTAssertFalse(popup.isOpen)
        popup.apply()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("one two".utf8))
    }

    func testCompactPopupAlignsWithLinkAndClosesOnViewRemoval() throws {
        let (_, surface, window) = try editor("prefix [label](https://example.com) suffix", type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 8, length: 0))
        window.makeFirstResponder(surface.editorView)
        surface.linkPopover.refresh()
        let popup = surface.linkPopover
        XCTAssertTrue(popup.isOpen)
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        let cluster = try XCTUnwrap(layout.clusters.first { $0.text_start == 7 })
        let rect = surface.editorView.viewRect(cluster.typographic_bounds)
        let screen = window.convertToScreen(surface.editorView.convert(rect, to: nil))
        XCTAssertEqual(popup.popupFrame.minX, screen.minX, accuracy: 1)
        XCTAssertLessThanOrEqual(popup.popupFrame.maxY, screen.minY)
        let field = NSTextField(string: "Another control")
        surface.view.addSubview(field)
        window.makeFirstResponder(field)
        XCTAssertFalse(popup.isOpen)
        surface.refreshPresentation()
        XCTAssertFalse(popup.isOpen, "Background refresh must not reopen an inactive editor popup")
        window.makeFirstResponder(surface.editorView)
        XCTAssertTrue(popup.isOpen)
        surface.view.removeFromSuperview()
        XCTAssertFalse(popup.isOpen)
    }
}
