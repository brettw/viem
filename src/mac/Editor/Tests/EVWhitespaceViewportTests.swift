import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVWhitespaceViewportTests: XCTestCase {
    func testCachedHorizontalScrollRejectsCountedMarkerBatchFromOldViewport() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-whitespace-viewport-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        try backend.read(source: Data((String(repeating: "word ", count: 100) + " ").utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 300, height: 200)
        _ = try session.sendText(":set nowrap")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        let before = try session.layoutExport()
        let state = try session.viewportState()
        var identity = before.info.identity
        var viewport = ViemLayoutRectV1(x: state.left, y: state.top,
            width: before.info.viewport_width, height: before.info.viewport_height)
        var count: UInt64 = 0
        XCTAssertEqual(viem_core_view_copy_whitespace_markers(backend.core, session.viewID,
            &identity, &viewport, nil, 0, &count), UInt32(VIEM_STATUS_BUFFER_TOO_SMALL))
        XCTAssertGreaterThan(count, 0)
        _ = try session.setViewportOrigin(left: 30, expected: state)
        let after = try session.layoutExport()
        XCTAssertTrue(after.info.identity.isSameLayout(as: before.info.identity),
            "Dense cached rows should keep their layout identity while horizontal clipping changes")
        XCTAssertNotEqual(after.whitespace.markers, before.whitespace.markers)
        var bytes = [UInt8](repeating: 0xAB, count: Int(count))
        let status = bytes.withUnsafeMutableBufferPointer {
            viem_core_view_copy_whitespace_markers(backend.core, session.viewID,
                &identity, &viewport, $0.baseAddress, UInt64($0.count), &count)
        }
        XCTAssertEqual(status, UInt32(VIEM_STATUS_STALE_REVISION))
        XCTAssertEqual(count, 0)
        XCTAssertTrue(bytes.allSatisfy { $0 == 0xAB })
        XCTAssertThrowsError(try session.whitespaceMarkersExport(identity: identity, expectedViewport: viewport))
        XCTAssertEqual(try session.whitespaceMarkersExport(identity: identity), after.whitespace)
        var resized = ViemCoreOutcomeV1()
        resized.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        XCTAssertEqual(viem_core_view_resize(backend.core, session.viewID, 0, 0, &resized), UInt32(VIEM_STATUS_OK))
        XCTAssertTrue(try session.layoutExport().whitespace.markers.isEmpty,
            "A collapsed pane still has a valid empty presentation")
    }

    func testMarkerViewportRegionsAreValidatedBeforeOutputWrites() {
        var identity = ViemLayoutSnapshotIdentityV1()
        identity.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotIdentityV1>.size)
        var viewport = ViemLayoutRectV1(x: 0, y: 0, width: 300, height: 200)
        let status = withUnsafeMutablePointer(to: &viewport) { pointer in
            viem_core_view_copy_whitespace_markers(0, 0, &identity, pointer,
                UnsafeMutableRawPointer(pointer).assumingMemoryBound(to: UInt8.self),
                UInt64(MemoryLayout<ViemLayoutRectV1>.size),
                UnsafeMutableRawPointer(pointer).assumingMemoryBound(to: UInt64.self))
        }
        XCTAssertEqual(status, UInt32(VIEM_STATUS_INVALID_ARGUMENT))
        XCTAssertEqual(viewport.width, 300)
        XCTAssertEqual(viewport.height, 200)
    }
}
