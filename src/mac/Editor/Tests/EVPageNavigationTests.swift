import AppKit
import CEvimCore
import EvimAppShell
import XCTest
@testable import EvimEditor

@MainActor final class EVPageNavigationTests: XCTestCase {
    private let source = (0..<80).map { index in
        "## Section \(index)\n\nA paragraph with **formatted words** that wraps across several visual rows.\n\n"
    }.joined()

    private func pageKey(down: Bool) throws -> NSEvent {
        let character = down ? "\u{f72d}" : "\u{f72c}"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.function],
            timestamp: 0, windowNumber: 0, context: nil, characters: character,
            charactersIgnoringModifiers: character, isARepeat: true, keyCode: down ? 121 : 116))
    }

    private func traverse(_ view: EVEditorSurfaceController, session: EVCoreViewSession, down: Bool, label: String) throws {
        let event = try pageKey(down: down)
        for step in 0..<600 {
            let cursor = view.viewPresentation.cursor_utf8_offset
            let top = try session.viewportState().top
            view.editorView.keyDown(with: event)
            let nextCursor = view.viewPresentation.cursor_utf8_offset
            let nextTop = try session.viewportState().top
            let snapshot = try XCTUnwrap(view.layoutSnapshot)
            guard session.lastOutcome.command_status == UInt32(EVIM_COMMAND_STATUS_COMPLETE) else {
                XCTFail("\(label) down=\(down) step=\(step) status=\(session.lastOutcome.command_status) cursor=\(cursor)→\(nextCursor) top=\(top)→\(nextTop) coverage=\(snapshot.info.coverage_hard_line_start)..<\(snapshot.info.coverage_hard_line_end)/\(snapshot.info.document_hard_line_count)")
                return
            }
            XCTAssertNotNil(view.formattedPointInfo(atUTF8Offset: Int(nextCursor)))
            XCTAssertNil(view.commandOutput)
            if cursor == nextCursor && abs(top - nextTop) < 0.001 {
                if down {
                    XCTAssertEqual(snapshot.info.coverage_hard_line_end, snapshot.info.document_hard_line_count, label)
                    let last = try XCTUnwrap(snapshot.rows.last)
                    XCTAssertGreaterThanOrEqual(nextTop + snapshot.info.viewport_height,
                                                last.y + last.line_advance - 0.5, label)
                } else {
                    XCTAssertEqual(snapshot.info.coverage_hard_line_start, 0, label)
                    XCTAssertLessThanOrEqual(nextTop, try XCTUnwrap(snapshot.rows.first).y + 0.5, label)
                }
                return
            }
        }
        XCTFail("\(label): repeated page navigation did not reach the document boundary")
    }

    private func check(type: String, flow: Bool, width: CGFloat, height: CGFloat, entry: String) throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: width, height: height)
        view.viewDidLayout()
        let session = try XCTUnwrap(view.session)
        if type == EVDocument.markdownSourceType { try session.setParagraphFlow(flow) }
        if !entry.isEmpty { _ = try session.sendText(entry) }
        view.refreshPresentation()
        let revision = try backend.revision()
        let mode = view.viewPresentation.mode
        let first = try XCTUnwrap(view.layoutSnapshot)
        XCTAssertLessThan(first.info.coverage_hard_line_end, first.info.document_hard_line_count)
        let label = "type=\(type) flow=\(flow) size=\(width)x\(height) entry=\(entry)"
        try traverse(view, session: session, down: true, label: label)
        try traverse(view, session: session, down: false, label: label)
        XCTAssertEqual(view.viewPresentation.mode, mode, label)
        XCTAssertEqual(try backend.revision(), revision)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(view.canUndo)
        XCTAssertFalse(view.canRedo)
    }

    func testRepeatedPhysicalPageKeysTraverseMarkdownAndSourceFlowBeyondPartialLayouts() throws {
        for (type, flow) in [(EVDocument.markdownType, false), (EVDocument.markdownSourceType, false), (EVDocument.markdownSourceType, true)] {
            for (width, height): (CGFloat, CGFloat) in [(340, 180), (720, 420)] {
                try check(type: type, flow: flow, width: width, height: height, entry: "")
            }
        }
    }

    func testRepeatedPageKeysPreserveInsertReplaceAndVisualModes() throws {
        for entry in ["i", "R", "v"] {
            try check(type: EVDocument.markdownType, flow: false, width: 420, height: 220, entry: entry)
        }
    }
}
