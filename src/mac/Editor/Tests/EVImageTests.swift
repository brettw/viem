import AppKit
import CViemCore
import ImageIO
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVImageTests: XCTestCase {
    private func editor(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 650, height: 500)
        let window = EVTestFocusWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, window)
    }

    private func preview(_ name: String, _ view: NSView) throws {
        guard let path = ProcessInfo.processInfo.environment["VIEM_IMAGE_PREVIEW_DIR"] else { return }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.25))
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: directory.appendingPathComponent(name + ".png"))
    }

    func testRemoteImageIsAtomicURLPlaceholderAndOnlyExplicitOpenLaunchesBrowser() throws {
        let source = "before ![A picture](https://example.invalid/photo.png) after"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        XCTAssertEqual(try backend.formattedText(), "before \u{FFFC} after")
        let image = try XCTUnwrap(surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster))
        XCTAssertEqual(image.text_end - image.text_start, 3)
        XCTAssertEqual(image.advance, 300, accuracy: 1)
        XCTAssertEqual(image.typographic_bounds.height, 64, accuracy: 1)
        var opened: [URL] = []
        surface.editorView.openLinkURL = { url, completion in opened.append(url); completion(nil) }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
        surface.imagePopover.refresh()
        XCTAssertTrue(surface.imagePopover.isOpen)
        XCTAssertFalse(surface.linkPopover.isOpen)
        XCTAssertNotNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)))
        XCTAssertTrue(opened.isEmpty)
        try preview("remote-image", surface.view)
        surface.imagePopover.destinationButton.performClick(nil)
        XCTAssertEqual(opened.map(\.absoluteString), ["https://example.invalid/photo.png"])
        XCTAssertFalse(surface.imagePopover.isOpen)
    }

    func testInsertEditRemoveAndUndoPreserveImageSourceInBothViews() throws {
        for type in [EVDocument.markdownSourceType, EVDocument.markdownType] {
            let (backend, surface, window) = try editor("before  after", type: type)
            defer { surface.imagePopover.close(); window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            surface.formattingToolbar.insertImage.performClick(nil)
            let popup = surface.imagePopover
            XCTAssertTrue(popup.isEditing)
            popup.textField.stringValue = ""
            popup.destinationField.stringValue = "local.png"
            popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
            popup.applyButton.performClick(nil)
            XCTAssertNil(surface.commandOutput)
            let inserted = try backend.serializedSource(typeName: type)
            XCTAssertEqual(String(decoding: inserted, as: UTF8.self), "before ![](<local.png>) after")
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            popup.refresh(); XCTAssertTrue(popup.isOpen)
            popup.editButton.performClick(nil)
            popup.destinationField.stringValue = "https://example.invalid/new.png"
            popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
            try preview("image-editor", try XCTUnwrap(popup.textField.window?.contentView))
            popup.applyButton.performClick(nil)
            XCTAssertNil(surface.commandOutput)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), inserted)
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            popup.refresh(); popup.removeButton.performClick(nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data("before  after".utf8))
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), inserted)
        }
    }

    func testLocalRasterLoadsAsynchronouslyAndKeepsAspectWithinDocumentWidth() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-image-\(UUID().uuidString).png")
        defer { try? FileManager.default.removeItem(at: file) }
        let context = try XCTUnwrap(CGContext(data: nil, width: 1200, height: 600, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(red: 0.12, green: 0.45, blue: 0.72, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: 1200, height: 600))
        context.setFillColor(CGColor(red: 0.95, green: 0.72, blue: 0.2, alpha: 1)); context.fill(CGRect(x: 80, y: 80, width: 400, height: 360))
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), nil)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        let (_, surface, window) = try editor("![Local picture](\(file.path))\n\nAfter", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        let deadline = Date(timeIntervalSinceNow: 4)
        while Date() < deadline {
            if let image = surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster), image.typographic_bounds.height > 64 { break }
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.02))
        }
        let image = try XCTUnwrap(surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster))
        XCTAssertGreaterThan(image.typographic_bounds.height, 64)
        XCTAssertLessThanOrEqual(image.advance, Float(surface.editorView.textViewportRect.width))
        XCTAssertEqual(image.advance / image.typographic_bounds.height, 2, accuracy: 0.01)
        try preview("local-image", surface.view)
        let rect = surface.editorView.viewRect(image.typographic_bounds)
        let point = surface.editorView.convert(NSPoint(x: rect.maxX - 5, y: rect.midY), to: nil)
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        surface.editorView.mouseDown(with: event)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, image.text_start, "Clicking the right half still selects the image")
        XCTAssertTrue(surface.imagePopover.isOpen)
    }

    func testSelectedImageCopiesPortableFallbackAndNativeBackspaceIsUndoable() throws {
        let source = "![alt](<local.png>)"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        surface.pasteboard = EVAppKitPasteboardAccess(pasteboard)
        let state = try backend.documentState()
        try surface.session?.selectImage(at: 0, documentID: state.document_id, revision: state.document_revision)
        surface.refreshPresentation()
        XCTAssertEqual(surface.selectedUTF8Ranges(), [0..<3])
        XCTAssertNotNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)),
            "An atomic image selection uses the open-box border and suppresses the thin insertion indicator")
        surface.perform(menuCommand: .copy, sender: nil)
        XCTAssertNil(surface.commandOutput, surface.statusBarState.message)
        XCTAssertEqual(pasteboard.string(forType: .string), source)
        XCTAssertNil(pasteboard.data(forType: .rtf), "Do not export an empty object replacement character as rich text")
        let json = try XCTUnwrap(pasteboard.data(forType: NSPasteboard.PasteboardType("com.viem.clipboard.fragment.v1")))
        XCTAssertEqual(try EVClipboardFragment.decode(json).plainText, "\u{FFFC}")
        let backspace = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "\u{7F}", charactersIgnoringModifiers: "\u{7F}", isARepeat: false, keyCode: 51))
        surface.editorView.keyDown(with: backspace)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data())
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testInsertionArrowNavigationSelectsImageAndCanLeaveItsSelection() throws {
        let (_, surface, window) = try editor("A![alt](https://example.invalid/a.png)B", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        func key(_ text: String, code: UInt16) throws {
            let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: text, charactersIgnoringModifiers: text,
                isARepeat: false, keyCode: code))
            surface.editorView.keyDown(with: event)
        }
        try key("i", code: 34)
        try key("\u{F703}", code: 124)
        XCTAssertEqual(surface.selectedUTF8Ranges(), [1..<4])
        XCTAssertNotNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)))
        XCTAssertTrue(surface.imagePopover.isOpen)
        try key("\u{F703}", code: 124)
        XCTAssertTrue(surface.selectedUTF8Ranges().isEmpty)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 4)
        XCTAssertNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)))
        try key("\u{F702}", code: 123)
        XCTAssertEqual(surface.selectedUTF8Ranges(), [1..<4])
    }

    func testNestedLinkedImagePrefersImagePopupAndRetainsEditingFocus() throws {
        let (_, surface, window) = try editor("[![alt](local.png)](https://example.invalid)", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); surface.linkPopover.close(); window.close() }
        surface.refreshPresentation()
        XCTAssertTrue(surface.imagePopover.isOpen)
        XCTAssertFalse(surface.linkPopover.isOpen)
        surface.imagePopover.editButton.performClick(nil)
        surface.refreshPresentation()
        XCTAssertTrue(surface.imagePopover.isEditing)
        XCTAssertFalse(surface.linkPopover.isOpen)
        surface.imagePopover.cancel()
        surface.linkPopover.openEditor()
        surface.refreshPresentation()
        XCTAssertTrue(surface.linkPopover.isEditing)
        XCTAssertFalse(surface.imagePopover.isOpen)
    }

    func testImageBesideCombiningTextKeepsAtomicGeometryAndSourceModeLiteral() throws {
        for type in [EVDocument.markdownSourceType, EVDocument.markdownType] {
            let source = "![alt](https://example.invalid/a.png)\u{0301}after"
            let (backend, surface, window) = try editor(source, type: type)
            defer { surface.imagePopover.close(); window.close() }
            if type == EVDocument.markdownType {
                let image = try XCTUnwrap(surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster))
                XCTAssertEqual(image.text_start, 0); XCTAssertEqual(image.text_end, 3)
                XCTAssertTrue(surface.layoutSnapshot?.clusters.contains { $0.text_start == 3 } == true)
            } else {
                XCTAssertEqual(try backend.formattedText(), source)
                XCTAssertFalse(surface.layoutSnapshot?.clusters.contains(where: surface.editorView.isImageCluster) == true)
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 20, length: 0))
                surface.imagePopover.refresh(); XCTAssertTrue(surface.imagePopover.isOpen)
            }
        }
    }
}
