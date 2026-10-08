import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVLinkPopoverTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var viemGeneration: UInt64 = 0
        var viemIsWritable = true
        var text = ""
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { !text.isEmpty }
        func viemClearContents() -> Int { text = ""; viemGeneration += 1; return Int(viemGeneration) }
        func viemSetString(_ string: String) -> Bool { text = string; return true }
    }

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

    private func preview(_ name: String, view: NSView) throws {
        guard let path = ProcessInfo.processInfo.environment["VIEM_LINK_PREVIEW_DIR"] else { return }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.25))
        let window = view.window
        let appearance = window?.appearance
        defer { window?.appearance = appearance }
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for (suffix, appearanceName) in [("light", NSAppearance.Name.aqua), ("dark", NSAppearance.Name.darkAqua)] {
            window?.appearance = NSAppearance(named: appearanceName)
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: directory.appendingPathComponent(name + "-" + suffix + ".png"))
        }
    }

    func testURLOnlyInsertionAndInvalidDestinationLeavesDraftAndSourceIntact() throws {
        let (backend, surface, window) = try editor("", type: EVDocument.markdownType)
        defer { surface.linkPopover.close(); window.close() }
        let popup = surface.linkPopover
        popup.openEditor()
        let validHeight = popup.popupFrame.height
        popup.destinationField.stringValue = "javascript:alert(1)"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        popup.applyButton.performClick(nil)
        XCTAssertTrue(popup.isEditing)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.2))
        XCTAssertGreaterThan(popup.popupFrame.height, validHeight, "Validation expands the compact form only when needed")
        try preview("link-editor-error", view: try XCTUnwrap(popup.textField.window?.contentView))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data())
        popup.destinationField.stringValue = "https://example.com"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.2))
        XCTAssertEqual(popup.popupFrame.height, validHeight, accuracy: 1, "Clearing an error removes its reserved space")
        popup.applyButton.performClick(nil)
        XCTAssertFalse(popup.isEditing)
        XCTAssertEqual(try backend.formattedText(), "https://example.com")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL), "Toolbar insertion preserves Normal mode")
    }

    func testToolbarUsesSelectedTextAndAppliesOneUndoableLinkInBothViews() throws {
        for type in [EVDocument.markdownSourceType, EVDocument.markdownType] {
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

    func testSourcePopupIncludesDestinationSyntaxAndCopyEditRemoveActions() throws {
        let original = "before [**label**](https://example.com/a%20b) after"
        let (backend, surface, window) = try editor(original, type: EVDocument.markdownSourceType)
        defer { surface.linkPopover.close(); window.close() }
        let session = try XCTUnwrap(surface.session)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 25, length: 0))
        window.makeFirstResponder(surface.editorView)
        surface.linkPopover.refresh()
        let popup = surface.linkPopover
        let context = try session.linkContext()
        XCTAssertEqual(context.link?.start, 7)
        XCTAssertEqual(context.link?.text, "label")
        XCTAssertTrue(popup.isOpen)
        XCTAssertFalse(popup.isEditing)
        XCTAssertTrue(window.firstResponder === surface.editorView)
        let pasteboard = Pasteboard()
        surface.pasteboard = pasteboard
        try preview("link-toolbar", view: try XCTUnwrap(popup.copyButton.window?.contentView))
        popup.copyButton.performClick(nil)
        XCTAssertEqual(pasteboard.text, "https://example.com/a%20b")
        popup.editButton.performClick(nil)
        XCTAssertTrue(popup.isEditing)
        XCTAssertEqual(popup.textField.stringValue, "label")
        XCTAssertEqual(popup.destinationField.stringValue, pasteboard.text)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.2))
        let formContent = try XCTUnwrap(popup.textField.window?.contentView)
        formContent.layoutSubtreeIfNeeded()
        for control in [popup.textField, popup.destinationField, popup.applyButton] {
            XCTAssertTrue(formContent.bounds.insetBy(dx: 1, dy: 1).contains(control.convert(control.bounds, to: formContent)),
                          "Native fields and footer must retain margins inside the fitted popup")
        }
        try preview("link-editor", view: formContent)
        popup.destinationField.stringValue = "#heading"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        popup.applyButton.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        let edited = try backend.serializedSource(typeName: EVDocument.markdownSourceType)
        XCTAssertTrue(String(decoding: edited, as: UTF8.self).contains("**label**"))
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 10, length: 0))
        popup.refresh()
        popup.removeButton.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data("before **label** after".utf8))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), edited)
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
