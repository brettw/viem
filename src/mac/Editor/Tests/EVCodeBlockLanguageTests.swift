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

    func testCodeLanguageMenuKeepsScrolledViewportAwayFromCaretDuringSyntaxRefresh() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-block-scroll-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (EVStyleTestFixtures.character("syntax:Keyword"), .characterSize, .float(40)),
        ])
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (EVStyleTestFixtures.block("Code Block"), .characterSize, .float(14)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = String(repeating: "before\n\n", count: 100)
            + "```rust\n" + String(repeating: "fn main() {}\n", count: 12) + "```\n\n"
            + String(repeating: "after\n\n", count: 100)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.codeType, filename: "notes.md", allowAutomaticCode: false)
        let codeStyles = try EVCodeStyleSession(configuration: configuration)
        XCTAssertEqual(try codeStyles.snapshot().definition(for: EVStyleTestFixtures.character("syntax:Keyword"))?.properties[.characterSize]?.declared, .float(40))
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
        surface.performInput { _ = try session.goToLine(202) }
        let codeLayout = try XCTUnwrap(surface.layoutSnapshot)
        let label = try XCTUnwrap(codeLayout.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0 })
        let row = try XCTUnwrap(codeLayout.rows.first { $0.row_index == label.row_index })
        let top = CGFloat(label.typographic_bounds.y - 60)
        surface.performInput { _ = try session.goToLine(1) }
        surface.requestVerticalViewport(top: top)
        for _ in 0..<30 {
            backend.pollSyntax()
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.005))
        }
        let before = surface.viewportState
        let caret = surface.viewPresentation.cursor_utf8_offset
        let mode = surface.viewPresentation.mode
        XCTAssertEqual(caret, 0)
        XCTAssertGreaterThan(before.top, 100)
        func assertScrollPreserved() {
            XCTAssertEqual(surface.viewportState.top, before.top, accuracy: 0.01)
            XCTAssertEqual(surface.viewportState.left, before.left, accuracy: 0.01)
            XCTAssertEqual(surface.editorView.viewportOrigin.y, CGFloat(before.top), accuracy: 0.01)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
            XCTAssertEqual(surface.viewPresentation.mode, mode)
        }
        try trackLanguagePicker(surface, keys: [(115, "\u{f729}"), (36, "\r")])
        assertScrollPreserved()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data(source.replacingOccurrences(of: "```rust", with: "```").utf8))
        // The reverse change also has a visible caret. Its delayed keyword
        // capture grows from the declared 14-point base to 40 points; applying
        // those metrics must not pin the caret baseline by changing scroll.
        surface.performInput { _ = try session.goToLine(202) }
        let visibleBefore = surface.viewportState
        let visibleCaret = surface.viewPresentation.cursor_utf8_offset
        XCTAssertEqual(visibleCaret, row.text_start)
        let visibleLayout = try XCTUnwrap(surface.layoutSnapshot)
        let visibleRow = try XCTUnwrap(visibleLayout.rows.first { $0.text_start == visibleCaret })
        XCTAssertGreaterThanOrEqual(visibleRow.baseline, visibleBefore.top)
        XCTAssertLessThanOrEqual(visibleRow.baseline, visibleBefore.top + visibleLayout.info.viewport_height)
        XCTAssertNotNil(visibleLayout.clusters.first { $0.text_start == visibleCaret },
            "the font publication assertion must start with an actually visible code token")
        func assertVisibleScrollPreserved() {
            XCTAssertEqual(surface.viewportState.top, visibleBefore.top, accuracy: 0.01)
            XCTAssertEqual(surface.viewportState.left, visibleBefore.left, accuracy: 0.01)
            XCTAssertEqual(surface.editorView.viewportOrigin.y, CGFloat(visibleBefore.top), accuracy: 0.01)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, visibleCaret)
            XCTAssertEqual(surface.viewPresentation.mode, mode)
        }
        let state = try backend.documentState()
        surface.performInput {
            _ = try session.setCodeBlockLanguage(documentID: state.document_id, revision: state.document_revision,
                offset: row.text_start, language: "rust")
        }
        assertVisibleScrollPreserved()
        var highlighted = false
        var lastSize: CGFloat?
        for _ in 0..<300 {
            backend.pollSyntax()
            assertVisibleScrollPreserved()
            if let cluster = surface.layoutSnapshot?.clusters.first(where: { $0.text_start == row.text_start }),
               let advance = session.provider.renderRegistry.enAdvance(identifier: cluster.render_run.identifier,
                   metricsGeneration: cluster.render_run.metrics_generation),
               advance > 0 {
                lastSize = advance * 2
                if abs(advance * 2 - 40) < 0.01 {
                    highlighted = true
                    break
                }
            }
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
        }
        XCTAssertTrue(highlighted, "the accepted delayed Rust keyword must use the fixture's 40-point syntax font; last native size: \(String(describing: lastSize))")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testDemoNoneToCLeavesVisibleFrameAndScrollingUsable() throws {
        @MainActor
        final class DocumentHost: EVDocumentHostEffectHandling {
            let url: URL
            init(_ url: URL) { self.url = url }
            func documentURL(for surface: any EVEditorSurface) -> URL? { url }
            func perform(documentHostRequests: [EVDocumentHostRequest],
                         completion: @escaping @MainActor (Result<String?, Error>) -> Void) {
                XCTFail("Changing code language must not issue document host effects")
                completion(.failure(EVDocumentHostError.unsupportedRequest))
            }
        }
        final class WheelEvent: NSEvent {
            let delta: CGFloat
            init(_ delta: CGFloat) { self.delta = delta; super.init() }
            @available(*, unavailable)
            required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
            override var type: NSEvent.EventType { .scrollWheel }
            override var scrollingDeltaX: CGFloat { 0 }
            override var scrollingDeltaY: CGFloat { delta }
            override var hasPreciseScrollingDeltas: Bool { true }
            override var phase: NSEvent.Phase { [] }
            override var momentumPhase: NSEvent.Phase { [] }
        }
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("docs/markdown_demo.md")
        let source = try Data(contentsOf: url)
        let bodies = ["Literal code inside this item.", "Quoted code keeps the Code Block style.",
            "Tilde fences work too.", "This fence is indented by two spaces.",
            "Four spaces start an indented code block.", "Preformatted HTML preserves"]
        let heights: [CGFloat] = [350, 1100]
        let scenarios: [(CGFloat, String)] = heights.flatMap { height in bodies.map { (height, $0) } }
        for (height, body) in scenarios {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-demo-language-\(UUID().uuidString)")
            addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
            let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
            try backend.read(source: source, typeName: EVDocument.markdownType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            let session = try XCTUnwrap(surface.session)
            let host = DocumentHost(url)
            surface.documentHostEffectHandler = host
            defer { withExtendedLifetime(host) {} }
            surface.loadViewIfNeeded()
            surface.view.frame = NSRect(x: 0, y: 0, width: 850, height: height)
            let window = EVTestFocusWindow(contentRect: surface.view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = surface.view
            defer { window.close() }
            surface.viewDidLayout()
            window.makeKeyAndOrderFront(nil)
            window.setKeyWindowForTesting(true)
            XCTAssertTrue(window.makeFirstResponder(surface.editorView))
            let formatted = try backend.formattedSnapshot()
            let text = try backend.formattedText(in: 0..<formatted.info.utf8_length, snapshot: formatted)
            let bodyRange = try XCTUnwrap(text.range(of: body))
            let codeOffset = UInt64(text[..<bodyRange.lowerBound].utf8.count)
            let context = "\(body), height \(height)"
            func assertVisibleFrame(_ phase: String) throws {
                let layout = try XCTUnwrap(surface.layoutSnapshot, "\(context), \(phase): \(surface.statusBarState.message)")
                let visible = CGRect(x: CGFloat(surface.viewportState.left), y: CGFloat(surface.viewportState.top),
                    width: CGFloat(layout.info.viewport_width), height: CGFloat(layout.info.viewport_height))
                XCTAssertTrue(layout.rows.contains { CGFloat($0.y) < visible.maxY && CGFloat($0.y + $0.ascent + $0.descent + $0.leading) > visible.minY }, "\(context), \(phase)")
                XCTAssertTrue(layout.clusters.contains { visible.intersects(CGRect(x: CGFloat($0.typographic_bounds.x),
                    y: CGFloat($0.typographic_bounds.y), width: CGFloat($0.typographic_bounds.width), height: CGFloat($0.typographic_bounds.height))) }, "\(context), \(phase)")
                XCTAssertNil(surface.commandOutput, "\(context), \(phase): \(surface.statusBarState.message)")
            }
            func visibleSelector(in snapshot: EVLayoutExport) -> ViemLayoutDecorationV1? {
                snapshot.decorations.first { item in
                    guard item.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0,
                          snapshot.rows.contains(where: { $0.row_index == item.row_index && $0.text_start == codeOffset }) else { return false }
                    let bounds = surface.editorView.viewRect(item.typographic_bounds)
                    return bounds.minY >= 30 && bounds.maxY < surface.editorView.textViewportRect.maxY - 30
                }
            }
            // Initial image decoding has its own geometry publication. Await
            // admitted jobs without stopping at unadmitted overscan images.
            var resourceWaitBudget = 200
            func settleResources() throws {
                var previousMetrics = session.provider.metricsGeneration
                var previousTop = surface.viewportState.top
                var stableTurns = 0
                while resourceWaitBudget > 0 && stableTurns < 2 {
                    resourceWaitBudget -= 1
                    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
                    backend.pollSyntax()
                    surface.refreshPresentation()
                    let current = try XCTUnwrap(surface.layoutSnapshot, context)
                    let metrics = session.provider.metricsGeneration
                    let top = surface.viewportState.top
                    if session.provider.pendingImageLoadCount == 0, metrics == previousMetrics, top == previousTop,
                       current.info.identity.metrics_generation == metrics,
                       session.backgroundLayout.isIdle, session.tableWidthRefinement.isIdle {
                        stableTurns += 1
                    } else {
                        stableTurns = 0
                    }
                    previousMetrics = metrics
                    previousTop = top
                }
                _ = try XCTUnwrap(stableTurns >= 2 ? true : nil,
                    "\(context): image geometry and current metrics must settle before the language action")
            }
            var label: ViemLayoutDecorationV1?
            // Build the same prefix-height and local-image caches as opening
            // the document and scrolling, with the caret remaining at its start.
            for _ in 0..<500 {
                if session.provider.pendingImageLoadCount > 0 { try settleResources() }
                var snapshot = try XCTUnwrap(surface.layoutSnapshot, "\(context): \(surface.statusBarState.message)")
                label = visibleSelector(in: snapshot)
                if label != nil {
                    try settleResources()
                    snapshot = try XCTUnwrap(surface.layoutSnapshot, context)
                    label = visibleSelector(in: snapshot)
                    if label != nil { break }
                }
                let firstVisible = snapshot.rows.first { $0.y + $0.ascent + $0.descent + $0.leading > surface.viewportState.top }
                let delta: CGFloat = firstVisible.map { $0.text_start > codeOffset ? 60 : -60 } ?? -60
                surface.editorView.scrollWheel(with: WheelEvent(delta))
                RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.005))
            }
            _ = try XCTUnwrap(label, "\(context): the target selector must become visible through scrolling")
            if height > 1000 {
                // Also retain a visible caret in the multi-block viewport.
                let utf16Offset = text[..<bodyRange.lowerBound].utf16.count
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: utf16Offset, length: 0))
            }
            try settleResources()
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let targetRow = try XCTUnwrap(snapshot.rows.first { $0.text_start == codeOffset }, "\(context): settled code row must remain materialized")
            let target = try XCTUnwrap(snapshot.decorations.first { item in
                item.flags & UInt32(VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0
                    && item.row_index == targetRow.row_index
            })
            let bounds = surface.editorView.viewRect(target.typographic_bounds)
            let location = surface.editorView.convert(NSPoint(x: bounds.midX, y: bounds.midY), to: nil)
            let exit = try XCTUnwrap(NSEvent.enterExitEvent(with: .mouseExited, location: location,
                modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil,
                eventNumber: 1, trackingNumber: 0, userData: nil))
            let tracker = MenuTracker(surface: surface, snapshot: snapshot, label: target, exit: exit)
            let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main) { note in
                MainActor.assumeIsolated {
                    if let menu = note.object as? NSMenu, menu.title == "Code block language" {
                        tracker.beganTracking(menu)
                    }
                }
            }
            defer { NotificationCenter.default.removeObserver(observer) }
            tracker.keyEvents = try [(UInt16(115), "\u{f729}"), (UInt16(8), "c"), (UInt16(36), "\r")].reversed().map { keyCode, characters in
                try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: location, modifierFlags: [], timestamp: 0,
                    windowNumber: window.windowNumber, context: nil, characters: characters,
                    charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
            }
            let timer = Timer(timeInterval: 0.05, repeats: false) { _ in MainActor.assumeIsolated { tracker.postKeys() } }
            RunLoop.current.add(timer, forMode: .eventTracking)
            defer { timer.invalidate() }
            let origin = surface.viewportState.top
            let caret = surface.viewPresentation.cursor_utf8_offset
            let down = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location,
                modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil,
                eventNumber: 1, clickCount: 1, pressure: 1))
            surface.editorView.mouseDown(with: down)
            XCTAssertNotNil(tracker.menu, context)
            XCTAssertEqual(tracker.highlighted.last, "C", context)
            for _ in 0..<30 {
                backend.pollSyntax()
                surface.refreshPresentation()
                try assertVisibleFrame("C")
                XCTAssertEqual(surface.viewportState.top, origin, accuracy: 0.05, "\(context): \(surface.statusBarState.message)")
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret, context)
                RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.005))
            }
            let changedSource = try backend.serializedSource(typeName: EVDocument.markdownType)
            XCTAssertNotEqual(changedSource, source, context)
            let changedFormatted = try backend.formattedSnapshot()
            XCTAssertEqual(try backend.formattedText(in: 0..<changedFormatted.info.utf8_length,
                snapshot: changedFormatted), text, "\(context): language edits must retain the complete body")
            surface.performInput { _ = try session.undo() }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source, context)
            surface.requestVerticalViewport(top: CGFloat(origin))
            try assertVisibleFrame("undo, restored code viewport")
            surface.performInput { _ = try session.redo() }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), changedSource, context)
            surface.requestVerticalViewport(top: CGFloat(origin))
            try assertVisibleFrame("redo, restored code viewport")
            for delta: CGFloat in [100, -50, -100] {
                surface.editorView.scrollWheel(with: WheelEvent(-delta))
                try assertVisibleFrame("subsequent wheel scroll")
            }
        }
    }
}
