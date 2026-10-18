import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemCoreTextProvider
@testable import ViemEditor

@MainActor
final class EVCodeBlockLanguageTests: XCTestCase {
    @MainActor
    private final class MenuTracker: NSObject, NSMenuDelegate {
        let surface: EVEditorSurfaceController
        let snapshot: EVLayoutExport
        let label: ViemLayoutDecorationV1
        let exit: NSEvent
        var keyEvents: [NSEvent] = []
        var menu: NSMenu?
        var highlighted: [String] = []
        init(surface: EVEditorSurfaceController, snapshot: EVLayoutExport, label: ViemLayoutDecorationV1, exit: NSEvent) {
            self.surface = surface; self.snapshot = snapshot; self.label = label; self.exit = exit
            super.init()
        }
        func beganTracking(_ menu: NSMenu) {
            self.menu = menu
            menu.delegate = self
            XCTAssertEqual(surface.editorView.codeBlockLanguageOpacity(for: label, in: snapshot), 1)
            surface.editorView.mouseExited(with: exit)
            XCTAssertEqual(surface.editorView.codeBlockLanguageOpacity(for: label, in: snapshot), 1,
                "an open native menu retains full opacity after the pointer exits")
        }
        func postKeys() {
            for event in keyEvents { NSApplication.shared.postEvent(event, atStart: true) }
        }
        func menu(_ menu: NSMenu, willHighlight item: NSMenuItem?) {
            if let item { highlighted.append(item.title) }
        }
    }

    @discardableResult
    private func trackLanguagePicker(_ surface: EVEditorSurfaceController, keys: [(UInt16, String)],
                                     enabled: Bool = true, selected: String = "Rust") throws -> NSMenu {
        let window = try XCTUnwrap(surface.editorView.window)
        window.contentView?.layoutSubtreeIfNeeded()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let rect = surface.editorView.viewRect(label.typographic_bounds)
        let location = surface.editorView.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil)
        let exit = try XCTUnwrap(NSEvent.enterExitEvent(with: .mouseExited, location: location,
            modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil,
            eventNumber: 1, trackingNumber: 0, userData: nil))
        let tracker = MenuTracker(surface: surface, snapshot: snapshot, label: label, exit: exit)
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main) { note in
            MainActor.assumeIsolated {
                if let menu = note.object as? NSMenu, menu.title == "Code block language" {
                    tracker.beganTracking(menu)
                }
            }
        }
        defer { NotificationCenter.default.removeObserver(observer) }
        tracker.keyEvents = try keys.reversed().map { keyCode, characters in
            try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: location, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
        }
        // NSMenu.popUp starts standalone tracking; unlike an NSComboBox button,
        // it has no native mouse-down to release before keyboard selection.
        // Deliver keys after AppKit enters tracking, so popup setup cannot
        // discard events queued before the standalone menu opens.
        let timer = Timer(timeInterval: 0.05, repeats: false) { _ in
            MainActor.assumeIsolated {
                tracker.postKeys()
            }
        }
        RunLoop.current.add(timer, forMode: .eventTracking)
        defer { timer.invalidate() }
        let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        surface.editorView.mouseDown(with: down)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
        surface.editorView.mouseExited(with: exit)
        XCTAssertEqual(surface.editorView.codeBlockLanguageOpacity(for: label, in: snapshot), 0.25, accuracy: 0.001,
            "closing the menu and leaving the selector restores idle opacity")
        let menu = try XCTUnwrap(tracker.menu, "clicking the language furniture must start the actual native menu")
        XCTAssertEqual(menu.items.first?.title, "None")
        XCTAssertTrue(menu.showsStateColumn)
        XCTAssertEqual(menu.items[1].title, "Obscure languages")
        let obscure = try XCTUnwrap(menu.items[1].submenu)
        XCTAssertTrue(obscure.showsStateColumn)
        XCTAssertTrue(menu.items[2].isSeparatorItem)
        XCTAssertEqual(menu.items.dropFirst(3).map(\.title), EVCodeLanguage.primaryLanguages.map(\.name))
        XCTAssertEqual(obscure.items.map(\.title), EVCodeLanguage.obscureLanguages.map(\.name))
        let choices = menu.items.filter { !$0.isSeparatorItem && $0.submenu == nil } + obscure.items
        XCTAssertEqual(choices.filter { $0.state == .on }.map(\.title), [selected])
        XCTAssertEqual(menu.items[1].state, obscure.items.contains { $0.state == .on } ? .on : .off)
        XCTAssertTrue((menu.items.filter { !$0.isSeparatorItem } + obscure.items).allSatisfy { $0.isEnabled == enabled },
            "language authoring choices must follow the document's editing policy")
        if keys.contains(where: { $0.0 == 36 }) {
            XCTAssertEqual(tracker.highlighted.last, "None", "native menu highlights: \(tracker.highlighted)")
        }
        return menu
    }

    func testNativeCodeLanguageMenuEscapeAndNoneSelectionPreserveCaretAndUndo() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-menu-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        let source = "before\n\n```rust\nfn main() {}\n```\n\nafter"
        try backend.read(source: Data(source.utf8), typeName: EVDocument.codeType, filename: "notes.md", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: 350)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 100, width: 850, height: 350),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        defer { window.close() }
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.formattingToolbar.formattedView.performClick(nil)
        let session = try XCTUnwrap(surface.session)
        let caret = surface.viewPresentation.cursor_utf8_offset
        let mode = surface.viewPresentation.mode
        let firstMenu = try trackLanguagePicker(surface, keys: [(53, "\u{1b}")])
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        let zig = try XCTUnwrap(firstMenu.items[1].submenu?.item(withTitle: "Zig"))
        XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(zig.action), to: zig.target, from: zig))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data(source.replacingOccurrences(of: "```rust", with: "```zig").utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        let capturedMenu = try trackLanguagePicker(surface, keys: [(53, "\u{1b}")], selected: "Zig")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("u").value)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        let staleZig = try XCTUnwrap(capturedMenu.items[1].submenu?.item(withTitle: "Zig"))
        XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(staleZig.action), to: staleZig.target, from: staleZig))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8),
            "overflow choices must reject the revision captured before undo")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        try trackLanguagePicker(surface, keys: [(115, "\u{f729}"), (36, "\r")])
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.replacingOccurrences(of: "```rust", with: "```").utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertEqual(surface.viewPresentation.mode, mode)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let start = Int(label.label_byte_start), end = start + Int(label.label_byte_length)
        XCTAssertEqual(String(decoding: snapshot.decorationLabels[start..<end], as: UTF8.self), "None ▾")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("u").value)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        try backend.setReadOnly(true)
        try trackLanguagePicker(surface, keys: [(53, "\u{1b}")], enabled: false)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testPreviewLanguageFurnitureAndLocalUndoableSelection() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        let source = Data("before\n\n```rust\nfn main() {}\n```\n\nafter".utf8)
        try backend.read(source: source, typeName: EVDocument.codeType, filename: "notes.md", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 700, height: 400)
        surface.formattingToolbar.formattedView.performClick(nil)
        let snapshot = try session.layoutExport()
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let start = Int(label.label_byte_start), end = start + Int(label.label_byte_length)
        XCTAssertEqual(String(decoding: snapshot.decorationLabels[start..<end], as: UTF8.self), "Rust ▾")
        let row = try XCTUnwrap(snapshot.rows.first { $0.row_index == label.row_index })
        XCTAssertLessThan(label.typographic_bounds.y, row.y + row.ascent + row.descent + row.leading)
        XCTAssertGreaterThan(label.typographic_bounds.y + label.typographic_bounds.height, row.y)
        let state = try backend.documentState()
        for _ in 0..<200 {
            backend.pollSyntax()
            if surface.layoutPaint?.runs.contains(where: { $0.text_start == row.text_start && $0.text_end >= row.text_start + 2 && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0 }) == true { break }
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
        }
        XCTAssertTrue(surface.layoutPaint?.runs.contains(where: { $0.text_start == row.text_start && $0.text_end >= row.text_start + 2 && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0 }) == true)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        _ = try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision, offset: row.text_start, language: "python")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(String(decoding: source, as: UTF8.self).replacingOccurrences(of: "```rust", with: "```python").utf8))
        XCTAssertThrowsError(try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision, offset: row.text_start, language: ""))
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("u").value)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    }

    func testFrontMatterLabelHasNoLanguagePickerOrHover() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-front-matter-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        let source = Data("---\nname: viem\n---\n\nbody".utf8)
        try backend.read(source: source, typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: 350)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 100, width: 850, height: 350),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = surface.view
        defer { window.close() }
        surface.viewDidLayout()
        window.makeKeyAndOrderFront(nil)
        window.setKeyWindowForTesting(true)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        XCTAssertNotEqual(label.flags & UInt32(VIEM_LAYOUT_DECORATION_LANGUAGE_PICKER_DISABLED), 0)
        let start = Int(label.label_byte_start), end = start + Int(label.label_byte_length)
        XCTAssertEqual(String(decoding: snapshot.decorationLabels[start..<end], as: UTF8.self), "front matter")
        XCTAssertFalse(EVCodeLanguage.all.contains { $0.id == "front matter" || $0.name == "front matter" })
        let rect = surface.editorView.viewRect(label.typographic_bounds)
        let location = surface.editorView.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil)
        let move = try XCTUnwrap(NSEvent.mouseEvent(with: .mouseMoved, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 0, pressure: 0))
        surface.editorView.mouseMoved(with: move)
        XCTAssertNil(surface.editorView.codeBlockLanguageHoverRect)
        XCTAssertEqual(surface.editorView.codeBlockLanguageOpacity(for: label, in: snapshot), 0.25)
        var pickerOpened = false
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main) { note in
            MainActor.assumeIsolated {
                if let menu = note.object as? NSMenu, menu.title == "Code block language" { pickerOpened = true }
            }
        }
        defer { NotificationCenter.default.removeObserver(observer) }
        // Bound the regression path if this disabled label accidentally opens a menu.
        let windowNumber = window.windowNumber
        let timer = Timer(timeInterval: 0.05, repeats: false) { _ in
            MainActor.assumeIsolated {
                guard let escape = NSEvent.keyEvent(with: .keyDown, location: location, modifierFlags: [], timestamp: 0,
                    windowNumber: windowNumber, context: nil, characters: "\u{1b}",
                    charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53) else {
                    XCTFail("Could not create Escape event to dismiss the language picker")
                    return
                }
                NSApplication.shared.postEvent(escape, atStart: true)
            }
        }
        RunLoop.current.add(timer, forMode: .eventTracking)
        defer { timer.invalidate() }
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: type, location: location, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0))
            if type == .leftMouseDown { surface.editorView.mouseDown(with: event) }
            else { surface.editorView.mouseUp(with: event) }
        }
        XCTAssertFalse(pickerOpened)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    }

    func testSelectorHoverPaintsBeneathCodeWithoutChangingGeometry() throws {
        let previousTheme = EVThemeStore.shared.theme
        var theme = previousTheme
        theme.background = EVThemeColor(1, 1, 1)
        EVThemeStore.shared.update(theme)
        defer { EVThemeStore.shared.update(previousTheme) }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-overlay-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (EVStyleTestFixtures.block("Code Block"), .characterSize, .float(14)),
            (EVStyleTestFixtures.block("Code Block"), .blockBackground, .color(EVStyleColor(red: 1, green: 1, blue: 1, alpha: 1))),
            (EVStyleTestFixtures.block("Code Block"), .blockPaddingTop, .float(0)),
            (EVStyleTestFixtures.block("Code Block"), .characterForeground, .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1))),
        ])
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (EVStyleTestFixtures.character("syntax:Comment"), .characterForeground, .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 1))),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        // Short words fill the first row into the selector instead of moving
        // an indivisible code token onto the next wrapped row.
        let source = "```rust\n// " + String(repeating: "x ", count: 100) + "\n```"
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let view = surface.editorView
        view.frame = NSRect(x: 0, y: 0, width: 500, height: 240)
        view.editingPreferences = EVEditingPreferences(configuration: configuration)
        view.editingPreferences.setCaretHoverEffect(false)
        _ = try XCTUnwrap(surface.session).resize(width: 500, height: 240)
        for _ in 0..<200 {
            backend.pollSyntax(); surface.refreshPresentation()
            if surface.layoutPaint?.runs.contains(where: { $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
                && $0.paint.foreground.red == 0 && $0.paint.foreground.green == 0 && $0.paint.foreground.blue == 0 }) == true { break }
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
        }
        surface.requestVerticalViewport(top: 0)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let rect = view.viewRect(label.typographic_bounds)
        func move(_ point: NSPoint) throws {
            view.mouseMoved(with: try XCTUnwrap(NSEvent.mouseEvent(with: .mouseMoved, location: view.convert(point, to: nil),
                modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 1, clickCount: 0, pressure: 0)))
        }
        func draw() throws -> NSBitmapImageRep {
            let context = try XCTUnwrap(CGContext(data: nil, width: 500, height: 240, bitsPerComponent: 8,
                bytesPerRow: 2000, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            context.translateBy(x: 0, y: 240)
            context.scaleBy(x: 1, y: -1)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
            view.draw(view.bounds)
            NSGraphicsContext.restoreGraphicsState()
            return NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
        }
        func bytes(_ image: NSBitmapImageRep) throws -> Data {
            Data(bytes: try XCTUnwrap(image.bitmapData), count: image.bytesPerRow * image.pixelsHigh)
        }
        try move(NSPoint(x: 1, y: 200))
        XCTAssertEqual(view.codeBlockLanguageOpacity(for: label, in: snapshot), 0.25, accuracy: 0.001)
        let idle = try draw()
        try move(NSPoint(x: rect.midX, y: rect.midY))
        XCTAssertEqual(view.codeBlockLanguageOpacity(for: label, in: snapshot), 1)
        let hovered = try draw()
        XCTAssertNotEqual(try bytes(idle), try bytes(hovered), "hover must repaint the actual selector")
        var blackCodePixels = 0
        for x in max(0, Int(rect.minX))..<min(500, Int(ceil(rect.maxX))) {
            for y in max(0, Int(rect.minY))..<min(240, Int(ceil(rect.maxY))) {
                if let color = idle.colorAt(x: x, y: y)?.usingColorSpace(.sRGB),
                   max(color.redComponent, color.greenComponent, color.blueComponent) < 0.01 {
                    blackCodePixels += 1
                    let repainted = try XCTUnwrap(hovered.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
                    XCTAssertLessThan(max(repainted.redComponent, repainted.greenComponent, repainted.blueComponent), 0.01,
                        "opaque code glyphs must cover the red selector beneath them")
                }
            }
        }
        XCTAssertGreaterThan(blackCodePixels, 0, "the native fixture must overlap selector and code glyphs")
        let exit = try XCTUnwrap(NSEvent.enterExitEvent(with: .mouseExited, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, eventNumber: 1, trackingNumber: 0, userData: nil))
        view.mouseExited(with: exit)
        XCTAssertEqual(view.codeBlockLanguageOpacity(for: label, in: snapshot), 0.25, accuracy: 0.001)
        XCTAssertEqual(try bytes(draw()), try bytes(idle), "exit restores the original rendered frame")
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: snapshot.info.identity))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

}
