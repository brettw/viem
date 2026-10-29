import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVCheckboxTests: XCTestCase {
    private final class FlippedCanvas: NSView {
        override var isFlipped: Bool { true }
    }

    private func surface(_ source: String, type: String? = nil)
        throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let type = type ?? EVDocument.markdownType
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-checkbox-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.readUntitled(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    func testTaskListToolbarAndMenuToggleUncheckedItems() throws {
        for source in ["Task", "- Task", "3. Task"] {
            for toolbar in [false, true] {
                let (backend, surface, _) = try surface(source)
                let document = EVDocument(editorBackend: backend)
                let window = EVDocumentWindowController(document: document, editorSurface: surface, configuration: backend.configuration)
                defer { window.close() }
                window.showWindow(nil)
                let button = try XCTUnwrap(surface.formattingToolbar.commandButtons[.taskList])
                XCTAssertNotNil(button.window, "the shell owns toolbar refreshes after editor state changes")
                surface.formattingToolbar.refresh()
                XCTAssertTrue(surface.presentation(for: .taskList).isEnabled)
                XCTAssertTrue(button.isEnabled)
                if toolbar { button.performClick(nil) }
                else { surface.perform(menuCommand: .taskList, sender: nil) }
                let expected = "- [ ] Task"
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(expected.utf8))
                XCTAssertEqual(try backend.formattedText(), "Task")
                XCTAssertTrue(surface.presentation(for: .taskList).isEnabled)
                XCTAssertEqual(surface.presentation(for: .taskList).state, .on)
                XCTAssertTrue(button.isEnabled)
                XCTAssertEqual(button.state, .on)
                XCTAssertEqual(surface.formattingToolbar.commandButtons[.bulletedList]?.state, .off)
                XCTAssertEqual(surface.formattingToolbar.commandButtons[.numberedList]?.state, .off)
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
                surface.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(expected.utf8))
                if toolbar { button.performClick(nil) }
                else { surface.perform(menuCommand: .taskList, sender: nil) }
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("Task".utf8))
                XCTAssertEqual(button.state, .off)
                XCTAssertNil(surface.commandOutput)
            }
        }
    }

    func testImportedTaskListIsExclusiveAndCanSwitchListKinds() throws {
        for source in ["- [X] Task", "3. [X] Task"] {
            for command in [EVMenuCommand.bulletedList, .numberedList, .taskList] {
                let (backend, surface, _) = try surface(source)
                surface.formattingToolbar.refresh()
                XCTAssertEqual(surface.presentation(for: .taskList).state, .on, source)
                XCTAssertEqual(surface.presentation(for: .bulletedList).state, .off, source)
                XCTAssertEqual(surface.presentation(for: .numberedList).state, .off, source)
                let button = try XCTUnwrap(surface.formattingToolbar.commandButtons[.taskList])
                XCTAssertEqual(button.state, .on)
                XCTAssertTrue(button.isEnabled)
                surface.perform(menuCommand: command, sender: nil)
                XCTAssertNil(surface.commandOutput)
                XCTAssertEqual(try backend.formattedText(), "Task")
                let changed = String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self)
                XCTAssertFalse(changed.contains("[X]"))
                XCTAssertEqual(surface.presentation(for: .taskList).state, .off)
                if command == .taskList { XCTAssertEqual(changed, "Task") }
                else { XCTAssertEqual(surface.presentation(for: command).state, .on) }
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            }
        }
    }

    func testTaskListMixedSelectionAppliesToAllItemsAndPreservesCheckedState() throws {
        let source = "- [X] One\n- Two\n- [ ] Three"
        let (backend, surface, _) = try surface(source)
        let document = EVDocument(editorBackend: backend)
        let window = EVDocumentWindowController(document: document, editorSurface: surface, configuration: backend.configuration)
        defer { window.close() }
        window.showWindow(nil)
        let text = try backend.formattedText() as NSString
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: text.length))
        let button = try XCTUnwrap(surface.formattingToolbar.commandButtons[.taskList])
        surface.formattingToolbar.refresh()
        XCTAssertTrue(button.isEnabled)
        XCTAssertEqual(button.state, .mixed)
        XCTAssertEqual(surface.presentation(for: .taskList).state, .mixed)
        XCTAssertEqual(surface.presentation(for: .bulletedList).state, .mixed)
        button.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- [X] One\n- [ ] Two\n- [ ] Three".utf8))
        XCTAssertEqual(button.state, .on)
        XCTAssertEqual(surface.presentation(for: .bulletedList).state, .off)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testTaskListAvailabilityRejectsLiteralCodeTablesAndNomodifiable() throws {
        for (source, type) in [("Task", EVDocument.plainTextType), ("Task", EVDocument.codeType),
                               ("```\nTask\n```", EVDocument.markdownType),
                               ("| Task |\n| ---- |", EVDocument.markdownType)] {
            let (backend, surface, _) = try surface(source, type: type)
            surface.formattingToolbar.refresh()
            XCTAssertFalse(surface.presentation(for: .taskList).isEnabled, source)
            XCTAssertFalse(try XCTUnwrap(surface.formattingToolbar.commandButtons[.taskList]).isEnabled, source)
            surface.perform(menuCommand: .taskList, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
        let (backend, surface, session) = try surface("Task")
        let document = EVDocument(editorBackend: backend)
        let window = EVDocumentWindowController(document: document, editorSurface: surface, configuration: backend.configuration)
        defer { window.close() }
        window.showWindow(nil)
        let button = try XCTUnwrap(surface.formattingToolbar.commandButtons[.taskList])
        try backend.setReadOnly(true)
        surface.refreshPresentation()
        XCTAssertTrue(surface.presentation(for: .taskList).isEnabled, "readonly controls saving, not editing")
        XCTAssertTrue(button.isEnabled)
        surface.performInput {
            _ = try session.sendText(":set noma")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertFalse(surface.presentation(for: .taskList).isEnabled)
        XCTAssertFalse(button.isEnabled)
    }

    func testNativeCheckboxSquareEndsAtBaselineAndScalesWithZoom() throws {
        let (_, surface, session) = try surface("- [ ] Task")
        var sizes: [Float] = []
        for scale in [CGFloat(1), 1.75] {
            surface.performInput { _ = try session.setScale(scale) }
            let snapshot = try session.layoutExport()
            let checkbox = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CHECKBOX) != 0 })
            let row = try XCTUnwrap(snapshot.rows.first { $0.row_index == checkbox.row_index })
            let bounds = checkbox.typographic_bounds
            XCTAssertGreaterThan(bounds.height, 0)
            XCTAssertEqual(bounds.width, bounds.height, accuracy: 0.001)
            XCTAssertEqual(bounds.y + bounds.height, row.baseline, accuracy: 0.001)
            sizes.append(bounds.height)
        }
        XCTAssertEqual(sizes[1], sizes[0] * 1.75, accuracy: 0.01)
    }

    func testPaintedCheckboxFillsTheCapHeightSquareInEveryAppearanceAndState() throws {
        _ = NSApplication.shared
        let scale: CGFloat = 2
        let padding: CGFloat = 8
        var drawings: [String: Data] = [:]
        for appearanceName in [NSAppearance.Name.aqua, .darkAqua] {
            for checked in [false, true] {
                for enabled in [false, true] {
                    for size in [CGFloat(12), 24, 36] {
                        let points = size + padding * 2
                        let pixels = Int(points * scale)
                        let stride = pixels * 4
                        let bitmap = try XCTUnwrap(CGContext(data: nil, width: pixels, height: pixels,
                            bitsPerComponent: 8, bytesPerRow: stride,
                            space: CGColorSpace(name: CGColorSpace.sRGB)!,
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue))
                        bitmap.translateBy(x: 0, y: CGFloat(pixels))
                        bitmap.scaleBy(x: scale, y: -scale)
                        let view = FlippedCanvas(frame: NSRect(x: 0, y: 0, width: points, height: points))
                        view.appearance = try XCTUnwrap(NSAppearance(named: appearanceName))
                        let target = NSRect(x: padding, y: padding, width: size, height: size)
                        NSGraphicsContext.saveGraphicsState()
                        NSGraphicsContext.current = NSGraphicsContext(cgContext: bitmap, flipped: true)
                        EVCheckboxDrawing.draw(in: target, checked: checked, enabled: enabled, view: view, context: bitmap)
                        NSGraphicsContext.restoreGraphicsState()

                        let bytes = try XCTUnwrap(bitmap.data).assumingMemoryBound(to: UInt8.self)
                        var maximumAlpha: UInt8 = 0
                        for pixel in 0..<(pixels * pixels) { maximumAlpha = max(maximumAlpha, bytes[pixel * 4 + 3]) }
                        let label = "\(appearanceName.rawValue), checked=\(checked), enabled=\(enabled), size=\(size)"
                        XCTAssertGreaterThan(maximumAlpha, 0, label)
                        if checked && enabled {
                            // A checked control needs a visible light checkmark,
                            // not merely an accent-colored filled square.
                            let innerStart = Int((padding + size * 0.16) * scale)
                            let innerEnd = Int((padding + size * 0.84) * scale)
                            var lightPixels = 0
                            for y in innerStart..<innerEnd {
                                for x in innerStart..<innerEnd {
                                    let offset = y * stride + x * 4
                                    if bytes[offset] > 235 && bytes[offset + 1] > 235
                                        && bytes[offset + 2] > 235 && bytes[offset + 3] > 240 {
                                        lightPixels += 1
                                    }
                                }
                            }
                            XCTAssertGreaterThan(lightPixels, Int(size * size * scale * scale * 0.04), label)
                        }
                        // The vector outline has no native-cell padding or shadow.
                        // Every painted pixel stays within the supplied square.
                        drawings[label] = Data(bytes: bytes, count: stride * pixels)
                        let threshold = 1
                        var minX = pixels, minY = pixels, maxX = -1, maxY = -1
                        for y in 0..<pixels {
                            for x in 0..<pixels where Int(bytes[y * stride + x * 4 + 3]) >= threshold {
                                minX = min(minX, x); minY = min(minY, y)
                                maxX = max(maxX, x); maxY = max(maxY, y)
                            }
                        }
                        let start = padding * scale, end = (padding + size) * scale
                        XCTAssertEqual(CGFloat(minX), start, label)
                        XCTAssertEqual(CGFloat(minY), start, label)
                        XCTAssertEqual(CGFloat(maxX + 1), end, label)
                        XCTAssertEqual(CGFloat(maxY + 1), end, "baseline edge: \(label)")
                        XCTAssertEqual(maxX - minX, maxY - minY, label)
                    }
                }
            }
        }
        for size in [CGFloat(12), 24, 36] {
            func drawing(_ appearance: NSAppearance.Name, _ checked: Bool, _ enabled: Bool) throws -> Data {
                try XCTUnwrap(drawings["\(appearance.rawValue), checked=\(checked), enabled=\(enabled), size=\(size)"])
            }
            XCTAssertNotEqual(try drawing(.aqua, false, true), try drawing(.darkAqua, false, true))
            for appearance in [NSAppearance.Name.aqua, .darkAqua] {
                XCTAssertNotEqual(try drawing(appearance, false, true), try drawing(appearance, true, true))
                for checked in [false, true] {
                    XCTAssertNotEqual(try drawing(appearance, checked, true), try drawing(appearance, checked, false))
                }
            }
        }
    }

    func testPointerTogglesNativeDrawingAndExactUndoWithoutMovingCaret() throws {
        let source = "- [ ] Task\n- Next"
        let (backend, surface, session) = try surface(source)
        try backend.setReadOnly(true)
        surface.refreshPresentation()
        let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 200, width: 600, height: 400),
                                       styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        window.setKeyWindowForTesting(true)
        defer { window.close() }
        window.makeFirstResponder(surface.editorView)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))
        let caret = surface.viewPresentation.cursor_utf8_offset
        let snapshot = try session.layoutExport()
        let checkbox = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CHECKBOX) != 0 })
        let rect = surface.editorView.viewRect(checkbox.typographic_bounds)
        let checkboxPoint = NSPoint(x: rect.midX, y: rect.midY)
        XCTAssertTrue(surface.editorView.pointerCursor(at: checkboxPoint) === NSCursor.arrow)
        func pixels() throws -> Data {
            let context = try XCTUnwrap(CGContext(data: nil, width: 600, height: 400, bitsPerComponent: 8,
                bytesPerRow: 2400, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            context.translateBy(x: 0, y: 400); context.scaleBy(x: 1, y: -1)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
            surface.editorView.draw(surface.editorView.bounds)
            NSGraphicsContext.restoreGraphicsState()
            let image = try XCTUnwrap(try XCTUnwrap(context.makeImage()).cropping(to: rect.insetBy(dx: -2, dy: -2).integral))
            return try XCTUnwrap(image.dataProvider?.data) as Data
        }
        let before = try pixels()
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown,
            location: surface.editorView.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil), modifierFlags: [],
            timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        surface.editorView.mouseDown(with: event)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("- [x] Task\n- Next".utf8))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
        XCTAssertTrue(surface.editorView.pointerCursor(at: checkboxPoint) === NSCursor.arrow)
        XCTAssertNotEqual(try pixels(), before, "the actual native checkbox changes its rendered state")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        XCTAssertEqual(try pixels(), before)
        XCTAssertThrowsError(try session.toggleCheckbox(paragraphID: checkbox.paragraph_id, expected: snapshot.info.identity),
                             "a checkbox click from an old layout is rejected")
    }
    func testCheckboxClickDoesNotCommitCompletionOrCompositionOverlay() throws {
        for composing in [false, true] {
            let source = "- [ ] apple apricot\n\nap"
            let (backend, surface, _) = try surface(source)
            let window = EVTestFocusWindow(contentRect: NSRect(x: 100, y: 200, width: 600, height: 400),
                                           styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentViewController = surface
            window.setKeyWindowForTesting(true)
            surface.editorView.applicationIsActive = { true }
            window.makeFirstResponder(surface.editorView)
            defer { window.close() }
            surface.editorView.insertText("GA", replacementRange: NSRange(location: NSNotFound, length: 0))
            if composing {
                surface.editorView.setMarkedText("draft", selectedRange: NSRange(location: 5, length: 0),
                    replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertTrue(surface.editorView.hasMarkedText())
            } else {
                let control = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                    modifierFlags: [.control], timestamp: 0, windowNumber: window.windowNumber, context: nil,
                    characters: "n", charactersIgnoringModifiers: "n", isARepeat: false, keyCode: 0))
                surface.editorView.keyDown(with: control)
                for _ in 0..<1000 where surface.completion?.isSearching == true { surface.pollCompletion() }
                XCTAssertTrue(surface.completion?.isActive == true)
            }
            let before = try backend.serializedSource(typeName: EVDocument.markdownType)
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let checkbox = try XCTUnwrap(snapshot.decorations.first { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_CHECKBOX) != 0 })
            let rect = surface.editorView.viewRect(checkbox.typographic_bounds)
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown,
                location: surface.editorView.convert(NSPoint(x: rect.midX, y: rect.midY), to: nil), modifierFlags: [],
                timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
            surface.editorView.mouseDown(with: event)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), before)
            if composing { XCTAssertTrue(surface.editorView.hasMarkedText()) }
            else { XCTAssertTrue(surface.completion?.isActive == true) }
            XCTAssertNil(surface.commandOutput)
        }
    }

}
