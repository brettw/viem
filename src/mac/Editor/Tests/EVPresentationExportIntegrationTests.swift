import AppKit
import CEvimCore
import XCTest

@testable import EvimEditor

final class EVPresentationExportIntegrationTests: XCTestCase {
    @MainActor
    func testModeCaretPathsAreExclusiveAndInactiveEditorUsesOutline() throws {
        let (surface, session, window) = try makeSurface(text: "abc")
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.editorView.applyPresentation()

        XCTAssertEqual(surface.editorView.customCaretPresentationForTesting, .active)
        XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)

        surface.performInput { _ = try session.sendText("i") }
        XCTAssertEqual(surface.editorView.customCaretPresentationForTesting, .hidden)
        XCTAssertTrue(surface.editorView.isDocumentInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)

        surface.performInput {
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
            _ = try session.sendText("R")
        }
        XCTAssertEqual(surface.editorView.customCaretPresentationForTesting, .active)
        XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)

        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(surface.editorView.customCaretPresentationForTesting, .inactiveOutline)
        XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testCommandLineExportAndNativeRenderStateTrackUTF8Cursor() throws {
        let (surface, session, window) = try makeSurface(text: "document")
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))

        surface.performInput {
            _ = try session.sendText(":")
            _ = try session.sendText("café")
        }

        let command = try XCTUnwrap(surface.commandLine)
        XCTAssertEqual(command.info.identity.kind, UInt32(EVIM_COMMAND_LINE_KIND_EX))
        XCTAssertEqual(command.prompt, ":")
        XCTAssertEqual(command.text, "café")
        XCTAssertEqual(command.info.utf8_length, UInt64("café".utf8.count))
        XCTAssertEqual(command.info.cursor_utf8_offset, UInt64("café".utf8.count))

        let atEnd = try XCTUnwrap(surface.editorView.commandLineRenderState())
        XCTAssertEqual(atEnd.prompt, ":")
        XCTAssertEqual(atEnd.text, "café")
        XCTAssertEqual(atEnd.displayText, ":café")
        XCTAssertEqual(atEnd.font.pointSize, 14, accuracy: 0.01)
        XCTAssertTrue(atEnd.bandRect.contains(atEnd.caretRect.origin))
        XCTAssertTrue(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible)

        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertTrue(surface.editorView.isInactiveCommandLineCaretOutlineVisible)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        XCTAssertTrue(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isInactiveCommandLineCaretOutlineVisible)

        surface.performInput { _ = try session.sendKey(kind: UInt32(EVIM_KEY_LEFT)) }

        let moved = try XCTUnwrap(surface.commandLine)
        XCTAssertEqual(moved.text, "café")
        XCTAssertEqual(moved.info.cursor_utf8_offset, UInt64("caf".utf8.count))
        let beforeFinalGrapheme = try XCTUnwrap(surface.editorView.commandLineRenderState())
        XCTAssertLessThan(beforeFinalGrapheme.caretRect.minX, atEnd.caretRect.minX)

        surface.performInput { _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE)) }
        XCTAssertNil(surface.editorView.commandLineRenderState())
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)

        for (input, prompt, kind): (String, String, UInt32) in [
            ("/", "/", UInt32(EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD)),
            ("?", "?", UInt32(EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD)),
        ] {
            surface.performInput { _ = try session.sendText(input) }
            XCTAssertEqual(surface.commandLine?.info.identity.kind, kind)
            XCTAssertEqual(surface.editorView.commandLineRenderState()?.prompt, prompt)
            surface.performInput { _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE)) }
        }

        surface.performInput { _ = try session.sendText("i") }
        XCTAssertTrue(surface.editorView.isDocumentInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testCharacterAndLineSelectionsUseExactLogicalAndLayoutExports() throws {
        let (surface, session, window) = try makeSurface(text: "ab\ncd")

        surface.performInput {
            _ = try session.sendText("v")
            _ = try session.sendText("l")
        }
        try assertSelection(
            surface,
            kind: UInt32(EVIM_VISUAL_SELECTION_KIND_CHARACTER),
            ranges: [0 ..< 2],
            text: "ab"
        )
        XCTAssertEqual((surface.editorView as NSTextInputClient).selectedRange(), NSRange(location: 0, length: 2))

        surface.performInput {
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
            _ = try session.sendText("V")
        }
        try assertSelection(
            surface,
            kind: UInt32(EVIM_VISUAL_SELECTION_KIND_LINE),
            ranges: [0 ..< 3],
            text: "ab\n"
        )
        XCTAssertTrue(try XCTUnwrap(surface.visualSelection).rectangles.contains { $0.rect.width == 0 })
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testBlockSelectionKeepsDiscontiguousSegmentsAndExactRectangles() throws {
        let (surface, session, window) = try makeSurface(text: "ab\ncd")

        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(EVIM_KEY_CONTROL_CHARACTER),
                codepoint: UInt32(Character("v").asciiValue!)
            )
        }
        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(EVIM_KEY_CHARACTER),
                codepoint: UInt32(Character("l").asciiValue!)
            )
        }
        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(EVIM_KEY_CHARACTER),
                codepoint: UInt32(Character("j").asciiValue!)
            )
        }

        try assertSelection(
            surface,
            kind: UInt32(EVIM_VISUAL_SELECTION_KIND_BLOCK),
            ranges: [0 ..< 2, 3 ..< 5],
            text: "ab\ncd"
        )
        XCTAssertNil(surface.selectedUTF8Range(), "a block must not become one destructive bounding range")
        XCTAssertEqual(surface.primarySelectedUTF8Range(), 0 ..< 2)
        XCTAssertEqual((surface.editorView as NSTextInputClient).selectedRange(), NSRange(location: 0, length: 2))

        let selection = try XCTUnwrap(surface.visualSelection)
        XCTAssertTrue(selection.segments.allSatisfy {
            $0.flags & UInt32(EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW) != 0
                && $0.flags & UInt32(EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE) != 0
                && $0.flags & UInt32(EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES) != 0
        })
        XCTAssertEqual(Set(selection.rectangles.map(\.row_index)), Set([0, 1]))
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testBidiSelectionPaintsEveryExactExportedRectangle() throws {
        let (surface, session, window) = try makeSurface(text: "abc אבג def")
        let revision = surface.viewPresentation.document_revision
        var start = EvimLayoutCaretPointV1()
        start.struct_size = UInt32(MemoryLayout<EvimLayoutCaretPointV1>.size)
        start.document_revision = revision
        start.text_offset = 2
        start.affinity = UInt32(EVIM_BOUNDARY_AFFINITY_DOWNSTREAM)
        var end = start
        end.text_offset = UInt64("abc אב".utf8.count)
        end.affinity = UInt32(EVIM_BOUNDARY_AFFINITY_UPSTREAM)

        surface.performInput {
            _ = try session.placeCursor(start, extendSelection: false)
            _ = try session.placeCursor(end, extendSelection: true)
        }

        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let exported = try XCTUnwrap(surface.visualSelection)
        XCTAssertEqual(exported.info.identity.kind, UInt32(EVIM_VISUAL_SELECTION_KIND_CHARACTER))
        XCTAssertFalse(exported.rectangles.isEmpty)
        XCTAssertEqual(
            surface.editorView.selectionRectsForDrawing(in: snapshot),
            exported.rectangles.map { surface.editorView.viewRect($0.rect) }
        )
        withExtendedLifetime(window) {}
    }

    @MainActor
    private func makeSurface(
        text: String
    ) throws -> (EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        let window = NSWindow(
            contentRect: surface.view.bounds,
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )
        window.contentView = surface.view
        return (surface, try XCTUnwrap(surface.session), window)
    }

    @MainActor
    private func assertSelection(
        _ surface: EVEditorSurfaceController,
        kind: UInt32,
        ranges: [Range<Int>],
        text: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot, file: file, line: line)
        let selection = try XCTUnwrap(surface.visualSelection, file: file, line: line)
        XCTAssertEqual(selection.info.identity.kind, kind, file: file, line: line)
        XCTAssertEqual(surface.selectedUTF8Ranges(), ranges, file: file, line: line)
        XCTAssertEqual(surface.selectionText(), text, file: file, line: line)
        XCTAssertEqual(
            surface.editorView.selectionRectsForDrawing(in: snapshot),
            selection.rectangles.map { surface.editorView.viewRect($0.rect) },
            file: file,
            line: line
        )
    }
}
