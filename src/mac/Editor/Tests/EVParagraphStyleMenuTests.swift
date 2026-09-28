import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVParagraphStyleMenuTests: XCTestCase {
    private func surface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: 520, height: 240)
        view.viewDidLayout()
        return (backend, view, try XCTUnwrap(view.session))
    }

    private func choose(_ id: String, in view: EVEditorSurfaceController) throws {
        let catalogue = try XCTUnwrap(view.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.role == .paragraph && $0.stableID == id })
        XCTAssertTrue(entry.presentation.isEnabled)
        let item = NSMenuItem(title: entry.displayName, action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)), keyEquivalent: "")
        item.representedObject = EVStyleMenuAction(kind: .assign, role: .paragraph, stableID: id,
            documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
            styleSheetRevision: catalogue.styleSheetRevision)
        XCTAssertTrue(view.editorView.validateMenuItem(item))
        view.editorView.performEditorStyleMenuAction(item)
        XCTAssertNil(view.commandOutput)
        XCTAssertEqual(view.currentStyleMenuCatalogue()?.entries.first { $0.stableID == id }?.presentation.state, .on)
    }

    private func currentListEntries(_ view: EVEditorSurfaceController) throws -> [EVStyleMenuEntry] {
        try XCTUnwrap(view.currentStyleMenuCatalogue()).entries.filter {
            $0.stableID.hasPrefix("BulletedList") || $0.stableID.hasPrefix("NumberedList")
        }
    }

    func testInternalListFamiliesRemainEditableAndOnlyCurrentStyleAppearsInMenu() throws {
        for (type, source) in [
            (EVDocument.markdownType, "- Parent\n  - Child\n\nOutside"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            let sheet = try backend.styleSheetSnapshot()
            let lists = sheet.definitions.filter { $0.flags.contains(.internalList) }
            let expectedIDs = Set(["BulletedList", "NumberedList"].flatMap { family in (1...4).map { "\(family)\($0)" } })
            XCTAssertEqual(Set(lists.map { $0.key.id.rawValue }), expectedIDs)
            XCTAssertTrue(lists.allSatisfy { !$0.flags.contains(.internalSyntax) })
            let editor = EVStyleEditorViewController()
            for definition in lists {
                editor.retarget(document: view, styleKey: definition.key)
                XCTAssertEqual(editor.inspection.selectedStyleKey, definition.key)
                XCTAssertEqual(editor.inspection.styleCount, sheet.definitions.count)
                XCTAssertTrue(editor.inspection.mutationsEnabled)
            }
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "BulletedList2")
            let entries = try currentListEntries(view)
            XCTAssertEqual(entries.map(\.stableID), ["BulletedList2"])
            XCTAssertEqual(entries.first?.presentation.state, .on)
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: 13, length: 0))
            XCTAssertTrue(try currentListEntries(view).isEmpty)
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 11))
            XCTAssertNil(try session.selectedNamedStyles().paragraph)
            XCTAssertTrue(try currentListEntries(view).isEmpty)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    func testListIndentCommandsFollowCoreCapabilitiesAndRoundTripExactSource() throws {
        for (type, source, family) in [
            (EVDocument.markdownType, "- Alpha\n- Beta", "BulletedList"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            XCTAssertFalse(view.presentation(for: .increaseIndent).isEnabled)
            XCTAssertFalse(view.presentation(for: .decreaseIndent).isEnabled)
            view.perform(menuCommand: .increaseIndent, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))
            XCTAssertTrue(view.presentation(for: .increaseIndent).isEnabled)
            XCTAssertFalse(view.presentation(for: .decreaseIndent).isEnabled)
            view.perform(menuCommand: .increaseIndent, sender: nil)
            XCTAssertNil(view.commandOutput)
            XCTAssertEqual(try backend.formattedText(), "Alpha\nBeta")
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "\(family)2")
            XCTAssertEqual(try currentListEntries(view).map(\.stableID), ["\(family)2"])
            XCTAssertTrue(view.presentation(for: .decreaseIndent).isEnabled)
            let indented = try backend.serializedSource(typeName: type)
            let (_, reopened, reopenedSession) = try surface(String(decoding: indented, as: UTF8.self), type: type)
            reopened.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))
            XCTAssertEqual(try reopenedSession.selectedNamedStyles().paragraph?.rawValue, "\(family)2")
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), indented)
            view.perform(menuCommand: .decreaseIndent, sender: nil)
            XCTAssertNil(view.commandOutput)
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "\(family)1")
            XCTAssertFalse(view.presentation(for: .decreaseIndent).isEnabled)
            let unindented = try backend.serializedSource(typeName: type)
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), indented)
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), unindented)
        }
        let (_, plain, _) = try surface("ordinary paragraph", type: "public.plain-text")
        XCTAssertFalse(plain.presentation(for: .increaseIndent).isEnabled)
        XCTAssertFalse(plain.presentation(for: .decreaseIndent).isEnabled)
        let (_, deepest, _) = try surface("- A\n  - B\n    - C\n      - D\n      - Deep", type: EVDocument.markdownType)
        deepest.editorView.setAccessibilitySelectedTextRange(NSRange(location: 8, length: 0))
        XCTAssertEqual(try currentListEntries(deepest).map(\.stableID), ["BulletedList4"])
        XCTAssertFalse(deepest.presentation(for: .increaseIndent).isEnabled)
        XCTAssertTrue(deepest.presentation(for: .decreaseIndent).isEnabled)
    }

    func testBlockQuoteMenuAssignmentPersistsAndUndoRedoRestoreExactSource() throws {
        for (type, source) in [
            (EVDocument.markdownType, "Words\n\nOutside"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            try choose("Block quote", in: view)
            XCTAssertEqual(try backend.formattedText(), "Words\nOutside")
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Block quote")
            let quoted = try backend.serializedSource(typeName: type)
            XCTAssertNotEqual(quoted, Data(source.utf8))
            let (_, reopened, reopenedSession) = try surface(String(decoding: quoted, as: UTF8.self), type: type)
            XCTAssertEqual(try reopenedSession.selectedNamedStyles().paragraph?.rawValue, "Block quote")
            XCTAssertEqual(reopened.currentStyleMenuCatalogue()?.entries.first { $0.stableID == "Block quote" }?.presentation.state, .on)
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), quoted)
        }
        let (_, plain, _) = try surface("Words", type: "public.plain-text")
        let quote = try XCTUnwrap(plain.currentStyleMenuCatalogue()?.entries.first { $0.stableID == "Block quote" })
        XCTAssertFalse(quote.presentation.isEnabled)
    }

    func testBlockQuoteMenuAtDocumentEndCreatesBlankQuoteReadyForTyping() throws {
        for (type, source, before, quotedText) in [
            (EVDocument.markdownType, "one **two**", "one two", "one two\n"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            for scalar in "GA".unicodeScalars {
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: scalar.value)
            }
            view.refreshPresentation()
            XCTAssertEqual(try backend.formattedText(), before)
            XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, UInt64(before.utf8.count))
            try choose("Block quote", in: view)
            XCTAssertEqual(try backend.formattedText(), quotedText)
            XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, UInt64(quotedText.utf8.count))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Block quote")
            let quoted = try backend.serializedSource(typeName: type)
            let (reopenedBackend, _, _) = try surface(String(decoding: quoted, as: UTF8.self), type: type)
            XCTAssertEqual(try reopenedBackend.formattedText(), quotedText)
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), quoted)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UnicodeScalar("i").value)
            _ = try session.sendText("العربية")
            XCTAssertEqual(try backend.formattedText(), quotedText + "العربية")
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Block quote")
        }
    }

    func testBlockQuoteBorderIsNonTextFurnitureAndCullsOutsideDirtyRegion() throws {
        let originalTheme = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(originalTheme) }
        for (type, source) in [
            (EVDocument.markdownType, "> Words\n\nOutside"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            for scale: CGFloat in [1, 2] {
                _ = try session.setScale(scale)
                view.refreshPresentation()
                let snapshot = try session.layoutExport()
                let border = try XCTUnwrap(snapshot.decorations.first {
                    $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0
                })
                XCTAssertEqual(border.label_byte_length, 0)
                XCTAssertGreaterThan(border.ink_bounds.width, 0)
                XCTAssertGreaterThan(border.ink_bounds.height, 0)
                let rect = view.editorView.viewRect(border.ink_bounds)
                XCTAssertFalse(view.editorView.listMarkersForDrawing(in: snapshot, dirtyRect: rect).isEmpty)
                XCTAssertTrue(view.editorView.listMarkersForDrawing(in: snapshot,
                    dirtyRect: NSRect(x: 490, y: 200, width: 20, height: 20)).isEmpty)
                let caret = try session.caretGeometry(offset: 0, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
                XCTAssertGreaterThan(caret.rect.x, border.ink_bounds.x + border.ink_bounds.width)
                let editor = view.editorView
                let context = try XCTUnwrap(CGContext(data: nil, width: Int(editor.bounds.width), height: Int(editor.bounds.height),
                    bitsPerComponent: 8, bytesPerRow: Int(editor.bounds.width) * 4, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
                editor.draw(editor.bounds)
                NSGraphicsContext.restoreGraphicsState()
                let image = NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
                let x = Int(rect.midX), y = Int(rect.midY)
                let pixels = [y, image.pixelsHigh - y - 1].compactMap {
                    image.colorAt(x: x, y: $0)?.usingColorSpace(.sRGB)
                }
                let background = try XCTUnwrap(EVThemeStore.shared.theme.background.color.usingColorSpace(.sRGB))
                let alpha = CGFloat(border.paint.foreground.alpha)
                let expectedRed = CGFloat(border.paint.foreground.red) * alpha + background.redComponent * (1 - alpha)
                let expectedGreen = CGFloat(border.paint.foreground.green) * alpha + background.greenComponent * (1 - alpha)
                let expectedBlue = CGFloat(border.paint.foreground.blue) * alpha + background.blueComponent * (1 - alpha)
                XCTAssertTrue(pixels.contains {
                    abs($0.redComponent - expectedRed) < 0.08
                        && abs($0.greenComponent - expectedGreen) < 0.08
                        && abs($0.blueComponent - expectedBlue) < 0.08
                }, "The exported quote border must be painted at its declared bounds")
            }
            XCTAssertEqual(try backend.formattedText(), "Words\nOutside")
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }
}
