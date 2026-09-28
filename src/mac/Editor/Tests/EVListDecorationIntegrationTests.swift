import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVListDecorationIntegrationTests: XCTestCase {
    private func makeSurface(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.frame = NSRect(x: 0, y: 0, width: 500, height: 240)
        _ = try XCTUnwrap(surface.session).resize(width: 500, height: 240)
        surface.refreshPresentation()
        return (backend, surface)
    }

    private func markerLabels(_ session: EVCoreViewSession) throws -> String {
        String(decoding: try session.layoutExport().decorationLabels, as: UTF8.self)
    }

    func testMarkersExportAsNonTextFurnitureIncludingEmptyItemsAndSourceToggle() throws {
        let source = "9. Alpha\n10. \n11. Third"
        let (backend, surface) = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        XCTAssertEqual(try backend.formattedText(), "Alpha\n\nThird")
        var snapshot = try session.layoutExport()
        XCTAssertEqual(String(decoding: snapshot.decorationLabels, as: UTF8.self), "9.10.11.")
        XCTAssertTrue(snapshot.decorations.allSatisfy { $0.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0 })
        let empty = try XCTUnwrap(snapshot.rows.first { $0.text_start == $0.text_end })
        XCTAssertEqual(empty.cluster_count, 0)
        XCTAssertGreaterThan(empty.caret_count, 0)
        let first = try XCTUnwrap(snapshot.decorations.first)
        let caret = try session.caretGeometry(offset: 0, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
        XCTAssertGreaterThan(caret.rect.x, first.x + first.advance)
        let hit = try session.hitTest(CGPoint(x: CGFloat(first.x), y: CGFloat(snapshot.rows[0].y + 2)), in: snapshot.info)
        XCTAssertEqual(hit.text_offset, 0)
        _ = try session.setScale(1.75)
        snapshot = try session.layoutExport()
        XCTAssertEqual(snapshot.decorations[0].font_size, 24.5, accuracy: 0.001)
        XCTAssertGreaterThan(snapshot.decorations[0].advance, first.advance)
        _ = try session.setMarkdownSource(true, expected: backend.documentState())
        XCTAssertTrue(try session.layoutExport().decorations.isEmpty)
        XCTAssertEqual(try backend.formattedText(), source)
        _ = try session.undo()
        XCTAssertEqual(String(decoding: try session.layoutExport().decorationLabels, as: UTF8.self), "9.10.11.")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testNativeTabAndShiftTabIndentInsideTheVisibleItem() throws {
        let (backend, surface) = try makeSurface("1. Parent\n2. **Body**")
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("jlli") }
        let original = try session.layoutExport()
        let originalX = try XCTUnwrap(original.decorations.last).x
        let cursor = surface.viewPresentation.cursor_utf8_offset
        XCTAssertEqual(try markerLabels(session), "1.2.")
        let tab = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "\t", charactersIgnoringModifiers: "\t", isARepeat: false, keyCode: 48))
        surface.editorView.keyDown(with: tab)
        XCTAssertGreaterThan(try XCTUnwrap(try session.layoutExport().decorations.last).x, originalX)
        XCTAssertEqual(try markerLabels(session), "1.a.")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        let backtab = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: .shift, timestamp: 0, windowNumber: 0, context: nil,
            characters: "\t", charactersIgnoringModifiers: "\t", isARepeat: false, keyCode: 48))
        surface.editorView.keyDown(with: backtab)
        XCTAssertEqual(try XCTUnwrap(try session.layoutExport().decorations.last).x, originalX, accuracy: 0.001)
        XCTAssertEqual(try markerLabels(session), "1.2.")
        surface.editorView.doCommand(by: #selector(NSResponder.insertTab(_:)))
        XCTAssertGreaterThan(try XCTUnwrap(try session.layoutExport().decorations.last).x, originalX)
        XCTAssertEqual(try markerLabels(session), "1.a.")
        surface.editorView.doCommand(by: #selector(NSResponder.insertBacktab(_:)))
        XCTAssertEqual(try XCTUnwrap(try session.layoutExport().decorations.last).x, originalX, accuracy: 0.001)
        XCTAssertEqual(try markerLabels(session), "1.2.")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(try backend.formattedText(), "Parent\nBody")
        XCTAssertFalse(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self).contains("\t"))
        XCTAssertNil(surface.commandOutput)
    }

    func testBackspaceAtNestedItemStartUnindentsThenRemovesTheTopLevelMarker() throws {
        let source = "1. Parent\n   1. Child\n      1. **Deep**"
        let (backend, surface) = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        let text = "Parent\nChild\nDeep"
        let deepStart = "Parent\nChild\n".utf8.count
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: deepStart, length: 0))
        surface.performInput { _ = try session.sendText("i") }
        let cursor = surface.viewPresentation.cursor_utf8_offset

        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList3")
        XCTAssertEqual(try markerLabels(session), "1.a.i.")

        surface.editorView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList2")
        XCTAssertEqual(try markerLabels(session), "1.a.b.")

        surface.editorView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList1")
        XCTAssertEqual(try markerLabels(session), "1.a.2.")

        surface.editorView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
        XCTAssertEqual(try markerLabels(session), "1.a.")
        XCTAssertNil(surface.commandOutput)

        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        let (reopened, reopenedSurface) = try makeSurface(String(decoding: saved, as: UTF8.self))
        reopenedSurface.editorView.setAccessibilitySelectedTextRange(NSRange(location: deepStart, length: 0))
        XCTAssertEqual(try reopened.formattedText(), text)
        XCTAssertEqual(try XCTUnwrap(reopenedSurface.session).selectedNamedStyles().paragraph?.rawValue, "Paragraph")
    }

    func testEscapeKeepsTheNativeCaretOnTheTerminalEmptyParagraph() throws {
        let (backend, surface) = try makeSurface("Body")
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("A") }
        surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        surface.editorView.cancelOperation(nil)
        XCTAssertEqual(try backend.formattedText(), "Body\n")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 5)
        let layout = try session.layoutExport()
        let empty = try XCTUnwrap(layout.rows.last)
        XCTAssertEqual(empty.text_start, 5)
        XCTAssertEqual(empty.text_end, 5)
        let caret = try session.caretGeometry(offset: surface.viewPresentation.cursor_utf8_offset,
            affinity: surface.viewPresentation.cursor_affinity, in: layout.info)
        XCTAssertGreaterThanOrEqual(caret.rect.y, empty.y)
        XCTAssertGreaterThan(caret.rect.height, 0)
        XCTAssertGreaterThan(try session.currentFontEnWidth(), 0)
    }

    func testNativeArrowReturnToAnEmptyItemKeepsBackspaceAtItsVisibleStart() throws {
        for source in ["- **Body**", "1. **Body**\n2. Tail"] {
            let (backend, surface) = try makeSurface(source)
            let session = try XCTUnwrap(surface.session)
            surface.performInput { _ = try session.sendText("A") }
            surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
            let start = surface.viewPresentation.cursor_utf8_offset
            let text = try backend.formattedText()
            surface.editorView.doCommand(by: #selector(NSResponder.moveUp(_:)))
            surface.editorView.doCommand(by: #selector(NSResponder.moveDown(_:)))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, start)
            surface.editorView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
            XCTAssertEqual(try backend.formattedText(), text)
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, start)
            XCTAssertNil(surface.commandOutput)
        }
    }

    func testMarkerDrawingCullsOutsideDirtyRegionAtOrdinaryZoomStops() throws {
        let (_, surface) = try makeSurface("- First body\n- Second body")
        let session = try XCTUnwrap(surface.session)
        for scale: CGFloat in [1, 1.25, 2] {
            _ = try session.setScale(scale)
            let snapshot = try session.layoutExport()
            let marker = try XCTUnwrap(snapshot.decorations.first)
            let markerRect = surface.editorView.viewRect(marker.ink_bounds).union(surface.editorView.viewRect(marker.typographic_bounds))
            XCTAssertFalse(surface.editorView.listMarkersForDrawing(in: snapshot, dirtyRect: markerRect).isEmpty)
            XCTAssertTrue(surface.editorView.listMarkersForDrawing(in: snapshot,
                dirtyRect: NSRect(x: 450, y: 200, width: 20, height: 20)).isEmpty)
            XCTAssertTrue(surface.editorView.listMarkersForDrawing(in: snapshot,
                dirtyRect: NSRect(x: markerRect.maxX + 2, y: markerRect.minY, width: 40, height: markerRect.height)).isEmpty)
        }
    }

    func testDecorationExportRejectsStaleIdentityAndDoesNotPartiallyCopy() throws {
        let (backend, surface) = try makeSurface("1. Alpha")
        let session = try XCTUnwrap(surface.session)
        let snapshot = try session.layoutExport()
        var identity = snapshot.info.identity
        var info = ViemLayoutDecorationsInfoV1()
        XCTAssertEqual(viem_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                         nil, 0, nil, 0, &info), UInt32(VIEM_STATUS_BUFFER_TOO_SMALL))
        XCTAssertGreaterThan(info.decoration_count, 0)
        var sentinel = ViemLayoutDecorationV1()
        sentinel.x = -1234
        XCTAssertEqual(viem_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                         &sentinel, 1, nil, 0, &info), UInt32(VIEM_STATUS_BUFFER_TOO_SMALL))
        XCTAssertEqual(sentinel.x, -1234)
        _ = try session.resize(width: 300, height: 240)
        XCTAssertNotEqual(viem_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                            nil, 0, nil, 0, &info), UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(info.decoration_count, 0)
    }

    func testNativeDrawingUsesMarkerParagraphColorWithoutMarkerSelection() throws {
        let theme = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(theme) }
        let (backend, surface) = try makeSurface("1. Body")
        let session = try XCTUnwrap(surface.session)
        let key = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "NumberedList1"))
        try session.editStyle(key: key, expected: backend.styleSheetSnapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(28)))
        try session.editStyle(key: key, expected: backend.styleSheetSnapshot().identity,
            mutation: .setDeclaration(.characterForeground, .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1))))
        surface.refreshPresentation()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let marker = try XCTUnwrap(snapshot.decorations.first)
        XCTAssertEqual(marker.paint.foreground.red, 1, accuracy: 0.001)
        XCTAssertEqual(marker.paint.foreground.green, 0, accuracy: 0.001)
        let view = surface.editorView
        let context = try XCTUnwrap(CGContext(data: nil, width: Int(view.bounds.width), height: Int(view.bounds.height),
            bitsPerComponent: 8, bytesPerRow: Int(view.bounds.width) * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        view.draw(view.bounds)
        NSGraphicsContext.restoreGraphicsState()
        let image = NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
        let rect = view.viewRect(marker.ink_bounds).insetBy(dx: -1, dy: -1)
        var redPixels = 0
        for y in max(0, Int(rect.minY))..<min(image.pixelsHigh, Int(ceil(rect.maxY))) {
            for x in max(0, Int(rect.minX))..<min(image.pixelsWide, Int(ceil(rect.maxX))) {
                // Bitmap contexts and AppKit view coordinates have opposite Y
                // origins; accept either representation, as the paint tests do.
                for pixelY in [y, image.pixelsHigh - y - 1] {
                    if let color = image.colorAt(x: x, y: pixelY)?.usingColorSpace(.sRGB),
                       color.redComponent > 0.5 && color.greenComponent < 0.35 && color.blueComponent < 0.35 { redPixels += 1 }
                }
            }
        }
        XCTAssertGreaterThan(redPixels, 0)
        surface.perform(menuCommand: .selectAll, sender: nil)
        XCTAssertTrue(view.selectionRectsForDrawing(in: try XCTUnwrap(surface.layoutSnapshot)).allSatisfy { $0.minX > rect.maxX })
    }
}
