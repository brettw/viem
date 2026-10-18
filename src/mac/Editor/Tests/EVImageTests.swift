import AppKit
import CViemCore
import CoreText
import ImageIO
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVImageTests: XCTestCase {

    private func editor(_ source: String, type: String, configuration: EVConfigurationStore? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, NSWindow) {
        let backend = configuration.map { EVCoreDocumentBackend(configuration: $0) } ?? EVCoreDocumentBackend()
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

    func testImageFileLocationsAreRelativeWithoutClimbingThroughRootAndRetainFilenameIdentity() throws {
        let document = URL(fileURLWithPath: "/work/project/docs/document.md")
        for (file, expected) in [
            ("/work/project/docs/image.png", "image.png"),
            ("/work/project/images/image.png", "../images/image.png"),
            ("/work/project/docs/image #?% cafe\u{301}:a.png", "image%20%23%3F%25%20cafe%CC%81%3Aa.png"),
            ("/work/project/docs/a%20.png", "a%2520.png"),
        ] {
            let url = URL(fileURLWithPath: file)
            let location = EVImageFileLocation.destination(for: url, relativeTo: document)
            XCTAssertEqual(location, expected)
            XCTAssertEqual(try EVLinkOpener.destinationURL(location, relativeTo: document).standardizedFileURL, url.standardizedFileURL)
        }
        let unrelated = URL(fileURLWithPath: "/elsewhere/image.png")
        let composed = try XCTUnwrap(URL(string: "file:///work/project/docs/caf%C3%A9.png"))
        XCTAssertEqual(EVImageFileLocation.destination(for: composed, relativeTo: document), "caf%C3%A9.png")
        XCTAssertEqual(EVImageFileLocation.destination(for: composed, relativeTo: nil), composed.absoluteString)
        XCTAssertEqual(EVImageFileLocation.destination(for: unrelated, relativeTo: document), unrelated.absoluteString)
        XCTAssertEqual(EVImageFileLocation.destination(for: unrelated, relativeTo: nil), unrelated.absoluteString)
        let prefixLookalike = URL(fileURLWithPath: "/work-other/image.png")
        XCTAssertEqual(EVImageFileLocation.destination(for: prefixLookalike, relativeTo: document), prefixLookalike.absoluteString)
        XCTAssertEqual(EVImageFileLocation.destination(for: URL(fileURLWithPath: "/images/image.png"),
            relativeTo: URL(fileURLWithPath: "/document.md")), "images/image.png")
    }

    func testImagePopupGrowthClampsToScreenBottomWithoutFlipping() {
        for visible in [NSRect(x: 0, y: 0, width: 1000, height: 700),
                        NSRect(x: -1400, y: -900, width: 1200, height: 800)] {
            let anchor = NSRect(x: visible.minX + 80, y: visible.minY + 240, width: 2, height: 20)
            func frame(_ height: CGFloat) -> NSRect {
                EVInlineContentPopoverController.imagePopupFrame(anchor: anchor,
                    size: NSSize(width: 388, height: height), visible: visible)
            }
            let compact = frame(160), expanded = frame(300), taller = frame(350)
            XCTAssertEqual(compact.maxY, anchor.minY - 5)
            XCTAssertEqual(expanded.minY, visible.minY + 4)
            XCTAssertEqual(expanded.maxY - compact.maxY, 300 - 160 - (compact.minY - visible.minY - 4))
            XCTAssertEqual(taller.minY, expanded.minY)
            XCTAssertEqual(taller.maxY - expanded.maxY, 50)
            XCTAssertEqual(frame(160), compact, "Collapsing returns to the caret anchor")
            XCTAssertTrue(visible.contains(taller))
        }
    }

    func testImageDraftRefreshesKeepTheSamePanelFrameAndFocus() throws {
        let (backend, surface, window) = try editor("text", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        surface.formattingToolbar.insertImage.performClick(nil)
        let popup = surface.imagePopover
        let panel = try XCTUnwrap(popup.textField.window)
        let frame = popup.popupFrame
        let responder = panel.firstResponder
        func assertFullyVisible(file: StaticString = #filePath, line: UInt = #line) {
            var ancestor: NSView? = popup.imageSizeControls
            while let view = ancestor {
                XCTAssertEqual(view.alphaValue, 1, "Draft refreshes must not fade the form", file: file, line: line)
                ancestor = view.superview
            }
        }
        assertFullyVisible()
        for size in [nil, NSSize(width: 200, height: 100), nil] {
            popup.imageSizeControls.setNaturalSize(size)
            assertFullyVisible()
            popup.refresh()
            XCTAssertTrue(popup.isOpen && popup.isEditing)
            XCTAssertTrue(popup.textField.window === panel)
            XCTAssertTrue(panel.firstResponder === responder)
            XCTAssertEqual(popup.popupFrame, frame)
        }
        popup.imageSizeControls.disclosure.performClick(nil)
        assertFullyVisible()
        XCTAssertGreaterThan(popup.popupFrame.height, frame.height)
        let expanded = popup.popupFrame
        popup.imageSizeControls.setNaturalSize(NSSize(width: 200, height: 100))
        assertFullyVisible()
        popup.refresh()
        XCTAssertEqual(popup.popupFrame, expanded)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("text".utf8))
    }

    func testImageLocationShowsSpacesAndPreservesEscapesOnApplyAndAltTextEdit() throws {
        let destination = "folder%20name/image%20with%2520%23%3F.png"
        let source = "![alt](<\(destination)>)"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        let popup = surface.imagePopover
        surface.refreshPresentation()
        XCTAssertEqual(popup.destinationButton.title, "folder name/image with%2520%23%3F.png")
        popup.editButton.performClick(nil)
        XCTAssertEqual(popup.destinationField.stringValue, "folder name/image with%2520%23%3F.png")
        popup.applyButton.performClick(nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        popup.refresh(); popup.editButton.performClick(nil)
        popup.textField.stringValue = "new alt"
        popup.applyButton.performClick(nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("![new alt](<\(destination)>)".utf8))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testPickedImageShowsSpacesButSavesTheEncodedFilename() throws {
        let (backend, surface, window) = try editor("", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        surface.formattingToolbar.insertImage.performClick(nil)
        let popup = surface.imagePopover
        let file = URL(fileURLWithPath: "/tmp/a space %20 #.png")
        popup.imageFilePanelPresenter = { _, _, completion in completion(file) }
        popup.chooseImageButton.performClick(nil)
        XCTAssertEqual(popup.destinationField.stringValue, EVImageFileLocation.display(file.absoluteString))
        XCTAssertTrue(popup.destinationField.stringValue.contains("a space %2520 %23.png"))
        popup.applyButton.performClick(nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("![](<\(file.absoluteString)>)".utf8))
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
        XCTAssertFalse(surface.imagePopover.reloadButton.isHidden)
        XCTAssertFalse(surface.imagePopover.reloadButton.isEnabled)
        XCTAssertFalse(surface.linkPopover.isOpen)
        XCTAssertNotNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)))
        XCTAssertTrue(opened.isEmpty)
        try preview("remote-image", surface.view)
        surface.imagePopover.destinationButton.performClick(nil)
        XCTAssertEqual(opened.map(\.absoluteString), ["https://example.invalid/photo.png"])
        XCTAssertFalse(surface.imagePopover.isOpen)
    }

    func testInsertEditRemoveAndUndoPreserveImageSourceInBothViews() throws {
        for type in [EVDocument.markdownType] {
            let (backend, surface, window) = try editor("before  after", type: type)
            defer { surface.imagePopover.close(); window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            surface.formattingToolbar.insertImage.performClick(nil)
            let popup = surface.imagePopover
            XCTAssertTrue(popup.isEditing)
            XCTAssertEqual(popup.applyButton.title, "Insert")
            popup.textField.stringValue = ""
            popup.destinationField.stringValue = "local.png"
            popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
            popup.applyButton.performClick(nil)
            XCTAssertNil(surface.commandOutput)
            let inserted = try backend.serializedSource(typeName: type)
            XCTAssertEqual(String(decoding: inserted, as: UTF8.self), "before ![](<local.png>) after")
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            popup.refresh(); XCTAssertTrue(popup.isOpen)
            XCTAssertFalse(popup.reloadButton.isHidden)
            popup.editButton.performClick(nil)
            XCTAssertEqual(popup.applyButton.title, "Apply")
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

    func testInsertAndUpdateLocalImageLoadWithoutReopeningAndSizeEditorRoundTrips() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        func write(_ name: String, _ width: Int, _ height: Int) throws -> URL {
            let file = directory.appendingPathComponent(name)
            let pixels = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            pixels.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
            pixels.fill(CGRect(x: 0, y: 0, width: width, height: height))
            let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
            CGImageDestinationAddImage(writer, try XCTUnwrap(pixels.makeImage()), nil)
            XCTAssertTrue(CGImageDestinationFinalize(writer))
            return file
        }
        func wait(_ condition: () -> Bool) {
            let deadline = Date(timeIntervalSinceNow: 5)
            while !condition() && Date() < deadline { RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01)) }
            XCTAssertTrue(condition())
        }
        let first = try write("first.png", 200, 100)
        let second = try write("second.png", 180, 120)
        let (backend, surface, window) = try editor("before  after", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        func imageHeight() -> Float? {
            surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster)?.typographic_bounds.height
        }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
        surface.formattingToolbar.insertImage.performClick(nil)
        let popup = surface.imagePopover
        popup.destinationField.stringValue = first.path
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: popup.destinationField))
        popup.applyButton.performClick(nil)
        wait { imageHeight() == 100 }
        XCTAssertNil(surface.commandOutput)
        popup.refresh(); popup.editButton.performClick(nil)
        popup.destinationField.stringValue = second.path
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: popup.destinationField))
        popup.applyButton.performClick(nil)
        wait { imageHeight() == 120 }
        let beforeSize = try backend.serializedSource(typeName: EVDocument.markdownType)
        popup.refresh(); popup.editButton.performClick(nil)
        wait { popup.imageSizeControls.useImageSizeButton.isEnabled }
        popup.imageSizeControls.disclosure.performClick(nil)
        XCTAssertEqual(popup.imageSizeControls.widthField.stringValue, "180")
        XCTAssertEqual(popup.imageSizeControls.heightField.stringValue, "120")
        let sizes = popup.imageSizeControls
        sizes.widthField.stringValue = "90"
        sizes.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: sizes.widthField))
        XCTAssertEqual(sizes.heightField.stringValue, "60")
        try preview("image-size-editor", try XCTUnwrap(popup.textField.window?.contentView))
        popup.applyButton.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        wait { imageHeight() == 60 }
        let sized = try backend.serializedSource(typeName: EVDocument.markdownType)
        XCTAssertTrue(String(decoding: sized, as: UTF8.self).contains("width=\"90\" height=\"60\""))
        popup.refresh(); popup.editButton.performClick(nil)
        XCTAssertTrue(sizes.expanded)
        wait { sizes.useImageSizeButton.isEnabled }
        XCTAssertEqual(sizes.lockButton.state, .on)
        popup.applyButton.performClick(nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), sized)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), beforeSize)
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), sized)
    }

    func testHTMLImageDimensionsUseLocalRasterAndPaintTheirLayoutRectangle() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-html-image-\(UUID().uuidString).png")
        defer { try? FileManager.default.removeItem(at: file) }
        let context = try XCTUnwrap(CGContext(data: nil, width: 120, height: 60, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(try XCTUnwrap(CGColor(colorSpace: CGColorSpaceCreateDeviceRGB(), components: [1, 0, 0, 1])))
        context.fill(CGRect(x: 0, y: 0, width: 120, height: 60))
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), nil)
        XCTAssertTrue(CGImageDestinationFinalize(writer))

        for (attributes, width, height) in [
            ("width='240'", Float(240), Float(120)),
            ("height='160'", Float(320), Float(160)),
            ("width='80' height='180'", Float(80), Float(180)),
            ("width='1024' height='2048'", Float(512), Float(1024)),
        ] {
            let source = "<img src='\(file.path)' \(attributes) alt='preserved' title='untouched'>"
            let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
            defer { surface.imagePopover.close(); window.close() }
            XCTAssertEqual(try backend.formattedText(), "\u{FFFC}")
            let registry = try XCTUnwrap(surface.session?.provider.renderRegistry)
            func painted(_ image: ViemPositionedClusterV1, width: Int, height: Int) throws -> NSBitmapImageRep {
                let canvas = try XCTUnwrap(CGContext(data: nil, width: width + 4, height: height + 4,
                    bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                XCTAssertTrue(registry.drawInlineImage(identifier: image.render_run.identifier,
                    metricsGeneration: image.render_run.metrics_generation,
                    in: CGRect(x: 2, y: 2, width: width, height: height),
                    color: CGColor(gray: 0, alpha: 1), context: canvas))
                return NSBitmapImageRep(cgImage: try XCTUnwrap(canvas.makeImage()))
            }
            func isRed(_ bitmap: NSBitmapImageRep, x: Int, y: Int) -> Bool {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { return false }
                return color.redComponent > 0.95 && color.greenComponent < 0.05 && color.alphaComponent > 0.95
            }
            let deadline = Date(timeIntervalSinceNow: 5)
            var loaded = false
            while Date() < deadline {
                if let image = surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster) {
                    loaded = isRed(try painted(image, width: 4, height: 4), x: 3, y: 3)
                    if loaded { break }
                }
                RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
            }
            XCTAssertTrue(loaded, "The HTML image must reach the existing local raster provider")
            let image = try XCTUnwrap(surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster))
            XCTAssertEqual(image.advance, width, accuracy: 0.01, attributes)
            XCTAssertEqual(image.typographic_bounds.width, width, accuracy: 0.01, attributes)
            XCTAssertEqual(image.typographic_bounds.height, height, accuracy: 0.01, attributes)
            XCTAssertLessThanOrEqual(image.advance, Float(surface.editorView.textViewportRect.width))
            let bitmap = try painted(image, width: Int(image.typographic_bounds.width), height: Int(image.typographic_bounds.height))
            XCTAssertTrue(isRed(bitmap, x: 3, y: 3))
            XCTAssertTrue(isRed(bitmap, x: Int(width), y: Int(height)), "The raster fills both authored dimensions")
            XCTAssertEqual(bitmap.colorAt(x: 0, y: 0)?.alphaComponent, 0, "Painting remains inside the final image rectangle")
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            XCTAssertFalse(backend.persistenceState.isDirty)
        }
    }

    func testRemoteHTMLImageUsesExistingPopupAndPreservesAttributes() throws {
        let source = "before <img src='https://example.invalid/photo.png' width='240' height='120' alt='A picture' title='Original'> after"
        for type in [EVDocument.markdownType] {
            let (backend, surface, window) = try editor(source, type: type)
            defer { surface.imagePopover.close(); window.close() }
            if type == EVDocument.markdownType {
                XCTAssertEqual(try backend.formattedText(), "before \u{FFFC} after")
                let image = try XCTUnwrap(surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster))
                XCTAssertEqual(image.advance, 240, accuracy: 0.01)
                XCTAssertEqual(image.typographic_bounds.height, 120, accuracy: 0.01)
            }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            surface.imagePopover.refresh()
            XCTAssertTrue(surface.imagePopover.isOpen)
            XCTAssertFalse(surface.imagePopover.reloadButton.isHidden)
            XCTAssertFalse(surface.imagePopover.reloadButton.isEnabled)
            var opened: [URL] = []
            surface.editorView.openLinkURL = { url, completion in opened.append(url); completion(nil) }
            XCTAssertTrue(opened.isEmpty)
            surface.imagePopover.destinationButton.performClick(nil)
            XCTAssertEqual(opened.map(\.absoluteString), ["https://example.invalid/photo.png"])
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            XCTAssertFalse(backend.persistenceState.isDirty)
        }
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

    func testImageToolbarAtUnselectedInsertCaretInsertsNewImageBesideExistingObject() throws {
        let source = "A![old](old.png)B"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        let session = try XCTUnwrap(surface.session)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 0))
        surface.performInput { _ = try session.sendText("i") }
        let toolbar = surface.formattingToolbar
        toolbar.refresh()
        XCTAssertEqual(toolbar.insertImage.state, .off)
        XCTAssertTrue(toolbar.insertImage.isEnabled)
        toolbar.insertImage.performClick(nil)
        let popup = surface.imagePopover
        XCTAssertTrue(popup.isEditing)
        XCTAssertEqual(popup.textField.stringValue, "")
        XCTAssertEqual(popup.destinationField.stringValue, "")
        popup.textField.stringValue = "new"
        popup.destinationField.stringValue = "new.png"
        popup.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification))
        popup.applyButton.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
                       Data("A![new](<new.png>)![old](old.png)B".utf8))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testImageParagraphStyleControlsNativeLabelAndSourceFontsAndBoxes() throws {
        for type in [EVDocument.markdownType] {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-image-style-\(UUID().uuidString)")
            defer { try? FileManager.default.removeItem(at: directory) }
            let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
            let source = "![alt](https://example.invalid/image.png)"
            let (backend, surface, window) = try editor(source, type: type, configuration: configuration)
            defer { surface.imagePopover.close(); window.close() }
            let style = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Image"))
            let inspector = EVStyleEditorViewController()
            inspector.themeStore = EVThemeStore(configuration: configuration)
            inspector.retarget(document: surface, styleKey: style)
            XCTAssertTrue(inspector.inspection.blockTabEnabled)
            XCTAssertTrue(inspector.setPropertyForTesting(.characterFontFamilies, value: .stringList(["Helvetica"])))
            XCTAssertTrue(inspector.setPropertyForTesting(.characterSize, value: .float(30)))
            XCTAssertTrue(inspector.setPropertyForTesting(.blockPaddingLeft, value: .float(12)))
            XCTAssertTrue(inspector.setPropertyForTesting(.blockBorderLeftWidth, value: .float(3)))
            surface.refreshPresentation()
            let cluster = try XCTUnwrap(surface.layoutSnapshot?.clusters.first)
            let font = try XCTUnwrap(surface.session?.provider.renderRegistry.resolvedFont(
                identifier: cluster.render_run.identifier, metricsGeneration: cluster.render_run.metrics_generation))
            XCTAssertEqual(CTFontGetSize(font), 30, accuracy: 0.01)
            XCTAssertTrue((CTFontCopyFamilyName(font) as String).contains("Helvetica"))
            XCTAssertGreaterThanOrEqual(cluster.typographic_bounds.x, 15)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            try preview(type == EVDocument.markdownType ? "image-style-label" : "image-style-source", surface.view)
        }
    }

    func testNormalArrowsTraverseStandaloneImageInBothDirections() throws {
        let (_, surface, window) = try editor("before\n\n![alt](https://example.invalid/a.png)\n\nafter", type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 0))
        func arrow(_ right: Bool) throws {
            let text = right ? "\u{F703}" : "\u{F702}"
            surface.editorView.keyDown(with: try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: text, charactersIgnoringModifiers: text,
                isARepeat: false, keyCode: right ? 124 : 123)))
        }
        try arrow(true)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)
        XCTAssertNotNil(surface.editorView.selectedImageCluster(in: try XCTUnwrap(surface.layoutSnapshot)))
        try arrow(true)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 11)
        try arrow(false)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 7)
        try arrow(false)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 5)
    }

    func testReloadRefreshesChangedFileAndBrokenPlaceholderWithoutEditingSource() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-reload-ui-\(UUID().uuidString).png")
        defer { try? FileManager.default.removeItem(at: file) }
        func write(_ width: Int, _ height: Int) throws {
            let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            context.setFillColor(CGColor(red: 0.2, green: 0.5, blue: 0.7, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
            CGImageDestinationAddImage(destination, try XCTUnwrap(context.makeImage()), nil)
            XCTAssertTrue(CGImageDestinationFinalize(destination))
        }
        func wait(_ predicate: () -> Bool) {
            let deadline = Date(timeIntervalSinceNow: 5)
            while !predicate() && Date() < deadline { RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01)) }
            XCTAssertTrue(predicate())
        }
        try write(200, 100)
        let source = "![Reload fixture](\(file.path))"
        let (backend, surface, window) = try editor(source, type: EVDocument.markdownType)
        defer { surface.imagePopover.close(); window.close() }
        func imageHeight() -> Float? { surface.layoutSnapshot?.clusters.first(where: surface.editorView.isImageCluster)?.typographic_bounds.height }
        wait { imageHeight() == 100 }
        surface.imagePopover.refresh()
        XCTAssertFalse(surface.imagePopover.reloadButton.isHidden)
        XCTAssertTrue(surface.imagePopover.reloadButton.isEnabled)
        XCTAssertLessThan(surface.imagePopover.reloadButton.frame.minX, surface.imagePopover.removeButton.frame.minX)
        try preview("image-toolbar", try XCTUnwrap(surface.imagePopover.destinationButton.window?.contentView))
        let revision = try backend.documentState().document_revision
        try write(200, 160)
        surface.imagePopover.reloadButton.performClick(nil)
        wait { imageHeight() == 160 }
        XCTAssertEqual(try backend.documentState().document_revision, revision)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        try FileManager.default.removeItem(at: file)
        surface.imagePopover.reloadButton.performClick(nil)
        wait { imageHeight() == 64 }
        try preview("broken-image", surface.view)
        try write(200, 120)
        surface.imagePopover.reloadButton.performClick(nil)
        wait { imageHeight() == 120 }
        XCTAssertEqual(try backend.documentState().document_revision, revision)
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
        for type in [EVDocument.markdownType] {
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
