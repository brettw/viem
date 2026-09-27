import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVParagraphFlowIntegrationTests: XCTestCase {
    @MainActor
    func testSourceFlowMenuIsPerViewAndPreservesExactSelectionAndHistory() throws {
        for (type, source) in [
            (EVDocument.markdownSourceType, "**alpha**\nbeta\n\ngamma"),
        ] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: type)
            let first = try makeSurface(backend)
            let second = try makeSurface(backend)
            let firstSession = try XCTUnwrap(first.session)
            let secondSession = try XCTUnwrap(second.session)
            XCTAssertFalse(try firstSession.paragraphFlow())
            XCTAssertFalse(try secondSession.paragraphFlow())
            XCTAssertTrue(first.presentation(for: .flowParagraphs).isEnabled)
            XCTAssertEqual(first.presentation(for: .flowParagraphs).state, .off)

            let range = try XCTUnwrap(source.range(of: "beta"))
            let start = source[..<range.lowerBound].utf8.count
            var point = ViemLayoutCaretPointV1()
            point.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
            point.document_revision = first.viewPresentation.document_revision
            point.text_offset = UInt64(start)
            point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
            _ = try firstSession.placeCursor(point, extendSelection: false)
            point.text_offset += 4
            point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
            _ = try firstSession.placeCursor(point, extendSelection: true)
            first.refreshPresentation()
            let selection = try firstSession.listSelection()
            let before = try firstSession.refreshState()
            let secondBefore = try secondSession.refreshState()

            first.perform(menuCommand: .flowParagraphs, sender: nil)
            XCTAssertTrue(try firstSession.paragraphFlow())
            XCTAssertFalse(try secondSession.paragraphFlow())
            XCTAssertEqual(first.presentation(for: .flowParagraphs).state, .on)
            XCTAssertEqual(second.presentation(for: .flowParagraphs).state, .off)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            let after = try firstSession.refreshState()
            XCTAssertEqual(after.document_revision, before.document_revision)
            XCTAssertEqual(after.cursor_utf8_offset, before.cursor_utf8_offset)
            XCTAssertEqual(after.mode, before.mode)
            let selectedAfter = try firstSession.listSelection()
            XCTAssertEqual(selectedAfter.text_start, selection.text_start)
            XCTAssertEqual(selectedAfter.text_end, selection.text_end)
            XCTAssertEqual(selectedAfter.kind, selection.kind)
            XCTAssertEqual(try secondSession.refreshState().cursor_utf8_offset, secondBefore.cursor_utf8_offset)
            XCTAssertFalse(first.presentation(for: .undo).isEnabled, "Flow is a view option, not an undoable source edit")
            withExtendedLifetime((first, second)) {}
        }
    }

    @MainActor
    func testFlowedSourceInsertionUsesOriginalLogicalOffsetAndUndoKeepsFlowEnabled() throws {
        let source = "**alpha**\nbeta\n\ngamma"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
        let surface = try makeSurface(backend)
        let session = try XCTUnwrap(surface.session)
        try session.setParagraphFlow(true)
        var point = ViemLayoutCaretPointV1()
        point.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
        point.document_revision = session.lastOutcome.document_revision
        point.text_offset = 2
        point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        _ = try session.placeCursor(point, extendSelection: false)
        _ = try session.sendText("i")
        _ = try session.sendText("Z")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data("**Zalpha**\nbeta\n\ngamma".utf8))
        _ = try session.sendText("u")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
        XCTAssertTrue(try session.paragraphFlow())
    }

    @MainActor
    func testWYSIWYGFlowIsEffectiveAndLockedAndInvalidABIArgumentsAreInert() throws {
        for (type, source, expectedFlow) in [
            (EVDocument.markdownType, "alpha\nbeta", true),
            ("public.plain-text", "alpha\nbeta", false),
        ] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try makeSurface(backend)
            let session = try XCTUnwrap(surface.session)
            XCTAssertEqual(try session.paragraphFlow(), expectedFlow)
            XCTAssertFalse(surface.presentation(for: .flowParagraphs).isEnabled)
            XCTAssertEqual(surface.presentation(for: .flowParagraphs).state, expectedFlow ? .on : .off)
            let before = try session.refreshState()
            XCTAssertThrowsError(try session.setParagraphFlow(!expectedFlow))
            var outcome = ViemCoreOutcomeV1()
            outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
            XCTAssertEqual(viem_core_view_set_paragraph_flow(backend.core, session.viewID, 2, &outcome), UInt32(VIEM_STATUS_INVALID_ARGUMENT))
            XCTAssertEqual(viem_core_view_paragraph_flow(backend.core, session.viewID, nil), UInt32(VIEM_STATUS_INVALID_ARGUMENT))
            var value: UInt32 = 7
            XCTAssertNotEqual(viem_core_view_paragraph_flow(backend.core, UInt64.max, &value), UInt32(VIEM_STATUS_OK))
            XCTAssertEqual(try session.refreshState().document_revision, before.document_revision)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    @MainActor
    private func makeSurface(_ backend: EVCoreDocumentBackend) throws -> EVEditorSurfaceController {
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 420, height: 220)
        surface.view.layoutSubtreeIfNeeded()
        surface.refreshPresentation()
        return surface
    }
}
