import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

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
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
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
        XCTAssertEqual(command.info.identity.kind, UInt32(VIEM_COMMAND_LINE_KIND_EX))
        XCTAssertEqual(command.prompt, ":")
        XCTAssertEqual(command.text, "café")
        XCTAssertEqual(command.info.utf8_length, UInt64("café".utf8.count))
        XCTAssertEqual(command.info.cursor_utf8_offset, UInt64("café".utf8.count))

        let atEnd = try XCTUnwrap(surface.editorView.commandLineRenderState())
        XCTAssertEqual(atEnd.prompt, ":")
        XCTAssertEqual(atEnd.text, "café")
        XCTAssertEqual(atEnd.displayText, ":café")
        XCTAssertEqual(atEnd.cursorUTF8Offset, "café".utf8.count)
        let statusBar = try statusBar(in: window)
        let atEndCaret = try XCTUnwrap(statusBar.commandCaretRect())
        XCTAssertTrue(statusBar.bounds.contains(atEndCaret.origin))
        XCTAssertGreaterThan(atEndCaret.minX, EVStatusBarView.contentInset)
        XCTAssertTrue(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isDocumentInsertionIndicatorVisible)

        let focusWindow = try XCTUnwrap(window as? EVTestFocusWindow)
        focusWindow.setKeyWindowForTesting(false)
        XCTAssertTrue(window.firstResponder === surface.editorView)
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertTrue(surface.editorView.isInactiveCommandLineCaretOutlineVisible)
        focusWindow.setKeyWindowForTesting(true)
        XCTAssertTrue(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isInactiveCommandLineCaretOutlineVisible)

        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertTrue(surface.editorView.isInactiveCommandLineCaretOutlineVisible)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        XCTAssertTrue(surface.editorView.isCommandLineInsertionIndicatorVisible)
        XCTAssertFalse(surface.editorView.isInactiveCommandLineCaretOutlineVisible)

        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_LEFT)) }

        let moved = try XCTUnwrap(surface.commandLine)
        XCTAssertEqual(moved.text, "café")
        XCTAssertEqual(moved.info.cursor_utf8_offset, UInt64("caf".utf8.count))
        let beforeFinalGrapheme = try XCTUnwrap(surface.editorView.commandLineRenderState())
        XCTAssertEqual(beforeFinalGrapheme.cursorUTF8Offset, "caf".utf8.count)
        let movedCaret = try XCTUnwrap(statusBar.commandCaretRect())
        XCTAssertLessThan(movedCaret.minX, atEndCaret.minX)

        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertNil(surface.editorView.commandLineRenderState())
        XCTAssertFalse(surface.editorView.isCommandLineInsertionIndicatorVisible)

        for (input, prompt, kind): (String, String, UInt32) in [
            ("/", "/", UInt32(VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD)),
            ("?", "?", UInt32(VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD)),
        ] {
            surface.performInput { _ = try session.sendText(input) }
            XCTAssertEqual(surface.commandLine?.info.identity.kind, kind)
            XCTAssertEqual(surface.editorView.commandLineRenderState()?.prompt, prompt)
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
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
            kind: UInt32(VIEM_VISUAL_SELECTION_KIND_CHARACTER),
            ranges: [0 ..< 2],
            text: "ab"
        )
        XCTAssertEqual((surface.editorView as NSTextInputClient).selectedRange(), NSRange(location: 0, length: 2))

        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendText("V")
        }
        try assertSelection(
            surface,
            kind: UInt32(VIEM_VISUAL_SELECTION_KIND_LINE),
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
                kind: UInt32(VIEM_KEY_CONTROL_CHARACTER),
                codepoint: UInt32(Character("v").asciiValue!)
            )
        }
        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CHARACTER),
                codepoint: UInt32(Character("l").asciiValue!)
            )
        }
        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CHARACTER),
                codepoint: UInt32(Character("j").asciiValue!)
            )
        }

        try assertSelection(
            surface,
            kind: UInt32(VIEM_VISUAL_SELECTION_KIND_BLOCK),
            ranges: [0 ..< 2, 3 ..< 5],
            text: "ab\ncd"
        )
        XCTAssertNil(surface.selectedUTF8Range(), "a block must not become one destructive bounding range")
        XCTAssertEqual(surface.primarySelectedUTF8Range(), 0 ..< 2)
        XCTAssertEqual((surface.editorView as NSTextInputClient).selectedRange(), NSRange(location: 0, length: 2))

        let selection = try XCTUnwrap(surface.visualSelection)
        XCTAssertTrue(selection.segments.allSatisfy {
            $0.flags & UInt32(VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW) != 0
                && $0.flags & UInt32(VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE) != 0
                && $0.flags & UInt32(VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES) != 0
        })
        XCTAssertEqual(Set(selection.rectangles.map(\.row_index)), Set([0, 1]))
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testBidiSelectionPaintsEveryExactExportedRectangle() throws {
        let (surface, session, window) = try makeSurface(text: "abc אבג def")
        let revision = surface.viewPresentation.document_revision
        var start = ViemLayoutCaretPointV1()
        start.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
        start.document_revision = revision
        start.text_offset = 2
        start.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        var end = start
        end.text_offset = UInt64("abc אב".utf8.count)
        end.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)

        surface.performInput {
            _ = try session.placeCursor(start, extendSelection: false)
            _ = try session.placeCursor(end, extendSelection: true)
        }

        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let exported = try XCTUnwrap(surface.visualSelection)
        XCTAssertEqual(exported.info.identity.kind, UInt32(VIEM_VISUAL_SELECTION_KIND_CHARACTER))
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
        let window = EVTestFocusWindow(
            contentRect: surface.view.bounds,
            styleMask: [.titled],
            backing: .buffered,
            defer: false
        )
        // A real pane pairs the editor with a status line, which renders the
        // command line and owns its caret.
        let container = NSView(frame: surface.view.bounds)
        container.addSubview(surface.view)
        let statusBar = EVStatusBarView()
        container.addSubview(statusBar)
        NSLayoutConstraint.activate([
            container.widthAnchor.constraint(equalToConstant: 520),
            container.heightAnchor.constraint(equalToConstant: 260),
            statusBar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            statusBar.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            statusBar.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        statusBar.apply(surface.statusBarState)
        surface.statusBarStateDidChange = { [weak statusBar] state in
            statusBar?.apply(state)
            statusBar?.superview?.layoutSubtreeIfNeeded()
        }
        window.contentView = container
        container.layoutSubtreeIfNeeded()
        surface.editorView.applicationIsActive = { true }
        window.setKeyWindowForTesting(true)
        return (surface, try XCTUnwrap(surface.session), window)
    }

    @MainActor
    private func statusBar(in window: NSWindow) throws -> EVStatusBarView {
        try XCTUnwrap(window.contentView?.subviews.compactMap { $0 as? EVStatusBarView }.first)
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
