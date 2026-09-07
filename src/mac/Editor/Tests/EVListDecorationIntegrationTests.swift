import AppKit
import CEvimCore
import EvimAppShell
import XCTest
@testable import EvimEditor

@MainActor
final class EVListDecorationIntegrationTests: XCTestCase {
    private func makeSurface(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.frame = NSRect(x: 0, y: 0, width: 500, height: 240)
        _ = try XCTUnwrap(surface.session).resize(width: 500, height: 240)
        surface.refreshPresentation()
        return (backend, surface)
    }

    func testMarkersExportAsNonTextFurnitureIncludingEmptyItemsAndSourceToggle() throws {
        let source = "<ol start='9'><li>Alpha</li><li></li><li>Third</li></ol>"
        let (backend, surface) = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        XCTAssertEqual(try backend.formattedText(), "Alpha\n\nThird")
        var snapshot = try session.layoutExport()
        XCTAssertEqual(String(decoding: snapshot.decorationLabels, as: UTF8.self), "9.10.11.")
        XCTAssertTrue(snapshot.decorations.allSatisfy { $0.flags & UInt32(EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0 })
        let empty = try XCTUnwrap(snapshot.rows.first { $0.text_start == $0.text_end })
        XCTAssertEqual(empty.cluster_count, 0)
        XCTAssertGreaterThan(empty.caret_count, 0)
        let first = try XCTUnwrap(snapshot.decorations.first)
        let caret = try session.caretGeometry(offset: 0, affinity: UInt32(EVIM_BOUNDARY_AFFINITY_DOWNSTREAM), in: snapshot.info)
        XCTAssertGreaterThan(caret.rect.x, first.x + first.advance)
        let hit = try session.hitTest(CGPoint(x: CGFloat(first.x), y: CGFloat(snapshot.rows[0].y + 2)), in: snapshot.info)
        XCTAssertEqual(hit.text_offset, 0)
        _ = try session.setScale(1.75)
        snapshot = try session.layoutExport()
        XCTAssertEqual(snapshot.decorations[0].font_size, 24.5, accuracy: 0.001)
        XCTAssertGreaterThan(snapshot.decorations[0].advance, first.advance)
        _ = try session.setFormat(.htmlSource, expected: backend.documentState())
        XCTAssertTrue(try session.layoutExport().decorations.isEmpty)
        XCTAssertEqual(try backend.formattedText(), source)
        _ = try session.undo()
        XCTAssertEqual(String(decoding: try session.layoutExport().decorationLabels, as: UTF8.self), "9.10.11.")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
    }

    func testMarkerDrawingCullsOutsideDirtyRegionAtOrdinaryZoomStops() throws {
        let (_, surface) = try makeSurface("<ul><li>First body</li><li>Second body</li></ul>")
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
        let (backend, surface) = try makeSurface("<ol><li>Alpha</li></ol>")
        let session = try XCTUnwrap(surface.session)
        let snapshot = try session.layoutExport()
        var identity = snapshot.info.identity
        var info = EvimLayoutDecorationsInfoV1()
        XCTAssertEqual(evim_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                         nil, 0, nil, 0, &info), UInt32(EVIM_STATUS_BUFFER_TOO_SMALL))
        XCTAssertGreaterThan(info.decoration_count, 0)
        var sentinel = EvimLayoutDecorationV1()
        sentinel.x = -1234
        XCTAssertEqual(evim_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                         &sentinel, 1, nil, 0, &info), UInt32(EVIM_STATUS_BUFFER_TOO_SMALL))
        XCTAssertEqual(sentinel.x, -1234)
        _ = try session.resize(width: 300, height: 240)
        XCTAssertNotEqual(evim_core_view_copy_layout_decorations(backend.core, session.viewID, &identity,
                            nil, 0, nil, 0, &info), UInt32(EVIM_STATUS_OK))
        XCTAssertEqual(info.decoration_count, 0)
    }

    func testNativeDrawingUsesMarkerParagraphColorWithoutMarkerSelection() throws {
        let theme = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(theme) }
        let (_, surface) = try makeSurface("<ol><li style='font-size:28pt;color:#ff0000'>Body</li></ol>")
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
