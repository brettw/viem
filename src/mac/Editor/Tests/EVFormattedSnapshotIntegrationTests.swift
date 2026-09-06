import AppKit
import CEvimCore
@testable import EvimEditor
import XCTest

@MainActor
final class EVFormattedSnapshotIntegrationTests: XCTestCase {
    func testTextInputAndAccessibilityUseAggregateCountsAndOnDemandRanges() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("A😀B".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadView()
        let view = surface.editorView

        backend.resetFormattedAccessCounters()
        XCTAssertEqual(view.accessibilityNumberOfCharacters(), 4)
        XCTAssertEqual(backend.formattedAccessCounters.rangeReadCalls, 0)

        var actual = NSRange(location: NSNotFound, length: 0)
        let substring = view.attributedSubstring(
            forProposedRange: NSRange(location: 1, length: 2),
            actualRange: &actual
        )
        XCTAssertEqual(substring?.string, "😀")
        XCTAssertEqual(actual, NSRange(location: 1, length: 2))
        XCTAssertEqual(backend.formattedAccessCounters.rangeReadCalls, 1)
        XCTAssertEqual(backend.formattedAccessCounters.requestedUTF8Bytes, 4)
        XCTAssertEqual(backend.formattedAccessCounters.fullRangeReadCalls, 0)
        XCTAssertGreaterThan(backend.formattedAccessCounters.utf16ToUTF8BatchCalls, 0)

        XCTAssertEqual(view.accessibilityValue() as? String, "A😀B")
        XCTAssertEqual(backend.formattedAccessCounters.fullRangeReadCalls, 1)
    }

    func testBoundedSnapshotRangeMappingAndPointInfoStayIdentityBound() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(
            source: Data("A😀e\u{301}\nB".utf8),
            typeName: "public.plain-text"
        )

        let snapshot = try backend.formattedSnapshot()
        XCTAssertEqual(snapshot.info.utf8_length, 10)
        XCTAssertEqual(snapshot.info.utf16_length, 7)
        XCTAssertEqual(snapshot.info.hard_line_count, 2)
        XCTAssertEqual(
            try backend.formattedText(in: 1 ..< 5, snapshot: snapshot),
            "😀"
        )
        XCTAssertEqual(
            try backend.mapFormattedUTF8ToUTF16(
                [0, 1, 5, 6, 8, 10],
                snapshot: snapshot
            ),
            [0, 1, 3, 4, 5, 7]
        )
        XCTAssertEqual(
            try backend.mapFormattedUTF16ToUTF8(
                [0, 1, 3, 4, 5, 7],
                snapshot: snapshot
            ),
            [0, 1, 5, 6, 8, 10]
        )

        let point = try backend.formattedPointInfo(atUTF8Offset: 8, snapshot: snapshot)
        XCTAssertEqual(point.hard_line_index, 0)
        XCTAssertEqual(point.hard_line_start, 0)
        XCTAssertEqual(point.hard_line_end, 8)
        XCTAssertEqual(point.grapheme_column, 3)
        XCTAssertThrowsError(
            try backend.formattedPointInfo(atUTF8Offset: 6, snapshot: snapshot)
        )

        let session = try EVCoreViewSession(document: backend, width: 400, height: 200)
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: 0x69) // i
        _ = try session.sendText("Z")
        XCTAssertThrowsError(
            try backend.formattedText(in: 0 ..< 1, snapshot: snapshot)
        )
    }

    func testMillionShortLineRefreshResizeScrollAndEditReadOnlyBoundedProjectionWindows() throws {
        let backend = EVCoreDocumentBackend()
        let lineCount = 1_000_000
        try backend.read(
            source: Data(String(repeating: "x\n", count: lineCount).utf8),
            typeName: "public.plain-text"
        )
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadView()
        surface.view.frame = NSRect(x: 0, y: 0, width: 780, height: 520)
        surface.viewDidLayout()
        XCTAssertEqual(surface.formattedUTF8Length, lineCount * 2)
        XCTAssertEqual(surface.formattedUTF16Length, lineCount * 2)

        backend.resetFormattedAccessCounters()
        surface.refreshPresentation()
        assertOnlyBoundedPresentationReads(backend.formattedAccessCounters)

        backend.resetFormattedAccessCounters()
        surface.view.frame.size = NSSize(width: 850, height: 610)
        surface.viewDidLayout()
        assertOnlyBoundedPresentationReads(backend.formattedAccessCounters)

        backend.resetFormattedAccessCounters()
        surface.requestVerticalViewport(top: 250_000)
        XCTAssertGreaterThan(surface.viewportState.top, 0)
        assertOnlyBoundedPresentationReads(backend.formattedAccessCounters)

        guard let session = surface.session else {
            XCTFail("Expected an attached core view")
            return
        }
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: 0x69) // i
        backend.resetFormattedAccessCounters()
        surface.performInput { _ = try session.sendText("Z") }
        assertOnlyBoundedPresentationReads(backend.formattedAccessCounters)
    }

    private func assertOnlyBoundedPresentationReads(
        _ counters: EVFormattedAccessCounters,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(counters.legacyWholeCopyCalls, 0, file: file, line: line)
        XCTAssertEqual(counters.fullRangeReadCalls, 0, file: file, line: line)
        XCTAssertGreaterThan(counters.snapshotInfoCalls, 0, file: file, line: line)
        XCTAssertGreaterThan(counters.rangeReadCalls, 0, file: file, line: line)
        XCTAssertGreaterThan(counters.requestedUTF8Bytes, 0, file: file, line: line)
        XCTAssertLessThan(counters.maximumRangeReadBytes, 64 * 1024, file: file, line: line)
    }
}
