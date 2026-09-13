import AppKit
import CViemCore
import XCTest

@testable import ViemEditor

final class EVGiantLineLayoutTests: XCTestCase {
    @MainActor
    func testWrappedGiantWordRetainsBoundedNativeOverflowGeometry() throws {
        let source = String(repeating: "a", count: 256 * 1024)
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 140)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        _ = try session.setWrap(true)
        let layout = try session.layoutExport()
        XCTAssertEqual(layout.rows.count, 1, "an indivisible word remains one overflow row")
        XCTAssertLessThan(layout.clusters.count, 5000)
        XCTAssertLessThan(layout.carets.count, 10_000)
        XCTAssertEqual(layout.rows.first?.text_end, UInt64(source.utf8.count))
        XCTAssertTrue(layout.clusters.allSatisfy {
            session.provider.renderRegistry.contains(identifier: $0.render_run.identifier,
                metricsGeneration: $0.render_run.metrics_generation)
        })
    }

    @MainActor
    func testWrappedGiantWordAfterShortLinePreservesPrefixAndTail() throws {
        let source = "x\nbefore " + String(repeating: "a", count: 256 * 1024) + " after"
        let surface = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        _ = try session.setWrap(true)
        let layout = try session.layoutExport()
        XCTAssertEqual(layout.rows.count, 4)
        XCTAssertEqual(layout.rows.last?.text_end, UInt64(source.utf8.count))
        XCTAssertLessThan(layout.clusters.count, 5000)
        XCTAssertLessThan(layout.carets.count, 10_000)
        XCTAssertTrue(layout.clusters.allSatisfy {
            session.provider.renderRegistry.contains(identifier: $0.render_run.identifier,
                metricsGeneration: $0.render_run.metrics_generation)
        })
    }

    @MainActor
    func testNativeGiantLineKeepsExactVisibleShapingAndRefillsHorizontalGeometry() throws {
        let prefix = String(repeating: "a", count: 4095) + "fi AV e\u{301} שלום مرحبا 👩‍🚀 "
        let phrase = "fi AV e\u{301} שלום مرحبا 👩‍🚀 "
        let reference = try makeSurface(prefix + String(repeating: phrase, count: 300))
        let referenceSession = try XCTUnwrap(reference.session)
        let expected = try referenceSession.layoutExport()
        let source = prefix + String(repeating: phrase, count: 3500)
        let surface = try makeSurface(source)
        let session = try XCTUnwrap(surface.session)
        let original = try session.layoutExport()
        let width = try XCTUnwrap(original.rows.first).width
        XCTAssertLessThan(original.clusters.count, 10_000)
        XCTAssertLessThan(original.carets.count, 20_000)
        XCTAssertEqual(original.rows.count, 1)
        for cluster in original.clusters where cluster.text_end < 3000 {
            let match = try XCTUnwrap(expected.clusters.first { $0.text_start == cluster.text_start })
            XCTAssertEqual(cluster.text_end, match.text_end)
            XCTAssertEqual(cluster.x, match.x, accuracy: 0.02)
            XCTAssertEqual(cluster.advance, match.advance, accuracy: 0.02)
            XCTAssertEqual(cluster.bidi_level, match.bidi_level)
        }
        let boundaryCluster = try XCTUnwrap(expected.clusters.first {
            $0.text_start <= 4095 && 4095 < $0.text_end
        })
        _ = try session.setViewportOrigin(left: CGFloat(boundaryCluster.x))
        let boundaryWindow = try session.layoutExport()
        for cluster in boundaryWindow.clusters where cluster.text_start >= 4090 && cluster.text_end <= 8000 {
            let match = try XCTUnwrap(expected.clusters.first { $0.text_start == cluster.text_start })
            XCTAssertEqual(cluster.text_end, match.text_end)
            XCTAssertEqual(cluster.x, match.x, accuracy: 0.02)
            XCTAssertEqual(cluster.advance, match.advance, accuracy: 0.02)
            XCTAssertEqual(cluster.bidi_level, match.bidi_level)
        }
        let maximum = CGFloat(try session.viewportState().maximum_left)
        for left in [maximum / 2, maximum, 0] {
            _ = try session.setViewportOrigin(left: left)
            let moved = try session.layoutExport()
            XCTAssertEqual(try XCTUnwrap(moved.rows.first).width, width)
            XCTAssertLessThan(moved.clusters.count, 10_000)
            XCTAssertLessThan(moved.carets.count, 20_000)
            let row = try XCTUnwrap(moved.rows.first)
            let point = try session.hitTest(CGPoint(x: left + 100, y: CGFloat(row.y + 1)), in: moved.info)
            XCTAssertTrue(moved.clusters.allSatisfy {
                session.provider.renderRegistry.contains(identifier: $0.render_run.identifier,
                    metricsGeneration: $0.render_run.metrics_generation)
            })
            _ = try session.placeCursor(point, extendSelection: false)
        }
        _ = try session.sendText("$")
        let end = try session.layoutExport()
        XCTAssertTrue(end.clusters.contains { $0.text_end == UInt64(source.utf8.count) })
        XCTAssertLessThan(end.clusters.count, 10_000)
    }

    @MainActor
    private func makeSurface(_ source: String) throws -> EVEditorSurfaceController {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 140)
        surface.viewDidLayout()
        _ = try XCTUnwrap(surface.session).setWrap(false)
        return surface
    }
}
