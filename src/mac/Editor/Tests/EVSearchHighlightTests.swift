import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVSearchHighlightTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ source: String, settings: String = "set hlsearch incsearch") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController)
    {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-search-highlight-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data((settings + "\n").utf8).write(to: directory.appendingPathComponent("startup.viem"))
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        return (backend, try makeSurface(backend))
    }

    private func makeSurface(_ backend: EVCoreDocumentBackend) throws -> EVEditorSurfaceController {
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 640, height: 260)
        surface.viewDidLayout()
        return surface
    }

    private func finishSearch(_ surface: EVEditorSurfaceController) throws {
        let session = try XCTUnwrap(surface.session)
        for _ in 0..<2_000 {
            if try !session.searchWorkPending() { break }
            surface.pollSearch()
        }
        XCTAssertFalse(try session.searchWorkPending())
        XCTAssertFalse(surface.searchWorkPending)
        XCTAssertFalse(surface.isSearchPolling)
        XCTAssertNil(surface.commandOutput)
    }

    private func type(_ text: String, into surface: EVEditorSurfaceController) {
        surface.editorView.insertText(text, replacementRange: noReplacement)
    }

    private func submit(_ text: String, in surface: EVEditorSurfaceController) throws {
        type(text, into: surface)
        surface.performInput {
            _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        try finishSearch(surface)
    }

    private func hasMatchPaint(at offset: UInt64, in surface: EVEditorSurfaceController) throws -> Bool {
        let paint = try XCTUnwrap(surface.layoutPaint)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertTrue(paint.info.identity.isSameLayout(as: snapshot.info.identity))
        return paint.runs.contains {
            $0.text_start <= offset && offset < $0.text_end
                && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0
        }
    }

    private func assertMatchPaint(
        at offsets: [UInt64], highlighted: Bool, in surface: EVEditorSurfaceController,
        file: StaticString = #filePath, line: UInt = #line
    ) throws {
        for offset in offsets {
            XCTAssertEqual(try hasMatchPaint(at: offset, in: surface), highlighted,
                "Search paint at byte \(offset)", file: file, line: line)
        }
    }

    func testSubmittedSearchPublishesExactPaintWithoutChangingDocumentHistory() throws {
        let (backend, surface) = try fixture("hit gap hit")
        let before = try backend.recoverySnapshot()

        try submit("/hit", in: surface)

        XCTAssertTrue(try hasMatchPaint(at: 0, in: surface))
        XCTAssertTrue(try hasMatchPaint(at: 8, in: surface))
        XCTAssertFalse(try hasMatchPaint(at: 4, in: surface))
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
    }

    func testSearchOptionQueriesUseNativeOptionNames() throws {
        let (_, surface) = try fixture("text")
        type(":set hls? is?", into: surface)
        surface.performInput {
            _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertEqual(surface.commandOutput, "hlsearch\nincsearch")
    }

    func testIncrementalSearchDefaultsToOnWithoutStartupOverride() throws {
        let (_, surface) = try fixture("hit gap hit", settings: "")
        type(":set is?", into: surface)
        surface.performInput {
            _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertEqual(surface.commandOutput, "incsearch")
    }

    func testIncrementalSearchPaintsEveryCurrentMatchWithoutPersistentHighlights() throws {
        for accept in [false, true] {
            // Leave incsearch unspecified to exercise its default through the native path.
            let (backend, surface) = try fixture("hit hat hit hat", settings: "set nohlsearch")
            let before = try backend.recoverySnapshot()
            type("/h", into: surface)
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 4, 8, 12], highlighted: true, in: surface)
            try assertMatchPaint(at: [3, 7, 11], highlighted: false, in: surface)

            type("it", into: surface)
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 8], highlighted: true, in: surface)
            try assertMatchPaint(at: [4, 12], highlighted: false, in: surface)

            if accept {
                surface.performInput {
                    _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
                }
            } else {
                surface.editorView.cancelOperation(nil)
            }
            try finishSearch(surface)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            try assertMatchPaint(at: [0, 4, 8, 12], highlighted: false, in: surface)
            XCTAssertEqual(try backend.recoverySnapshot(), before)
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertFalse(surface.canUndo)
        }
    }

    func testIncrementalSearchRestoresPersistentPatternOnCancelAndCommitsItOnAccept() throws {
        for accept in [false, true] {
            let (_, surface) = try fixture("hit hat hit hat")
            try submit("/hit", in: surface)
            try assertMatchPaint(at: [0, 8], highlighted: true, in: surface)
            type("/hat", into: surface)
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 8], highlighted: false, in: surface)
            try assertMatchPaint(at: [4, 12], highlighted: true, in: surface)

            if accept {
                surface.performInput {
                    _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
                }
            } else {
                surface.editorView.cancelOperation(nil)
            }
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 8], highlighted: !accept, in: surface)
            try assertMatchPaint(at: [4, 12], highlighted: accept, in: surface)
        }
    }

    func testIncrementalSearchTemporarilyOverridesHighlightSuppression() throws {
        for accept in [false, true] {
            let (_, surface) = try fixture("hit hat hit hat")
            try submit("/hit", in: surface)
            try submit(":nohlsearch", in: surface)
            try assertMatchPaint(at: [0, 4, 8, 12], highlighted: false, in: surface)
            type("/hat", into: surface)
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 8], highlighted: false, in: surface)
            try assertMatchPaint(at: [4, 12], highlighted: true, in: surface)

            if accept {
                surface.performInput {
                    _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
                }
            } else {
                surface.editorView.cancelOperation(nil)
            }
            try finishSearch(surface)
            try assertMatchPaint(at: [0, 8], highlighted: false, in: surface)
            try assertMatchPaint(at: [4, 12], highlighted: accept, in: surface)
        }
    }

    func testSharedSearchOptionsRefreshInactivePaneWithoutMovingItsCursor() throws {
        let (backend, active) = try fixture("hit gap hit")
        let inactive = try makeSurface(backend)
        let originalCursor = inactive.viewPresentation.cursor_utf8_offset

        try submit("/hit", in: active)
        try finishSearch(inactive)
        XCTAssertTrue(try hasMatchPaint(at: 0, in: inactive))
        let priorRefresh = inactive.presentationRefreshCount

        try submit(":set nohlsearch", in: active)
        try finishSearch(inactive)

        XCTAssertGreaterThan(inactive.presentationRefreshCount, priorRefresh)
        XCTAssertFalse(try hasMatchPaint(at: 0, in: inactive))
        XCTAssertFalse(try hasMatchPaint(at: 8, in: inactive))
        XCTAssertEqual(inactive.viewPresentation.cursor_utf8_offset, originalCursor)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testIncrementalPreviewRevealsMatchAndEscapeRestoresScrolledViewport() throws {
        let source = (0..<500).map { $0 == 450 ? "distant needle" : "ordinary line \($0)" }.joined(separator: "\n")
        let (backend, surface) = try fixture(source, settings: "set nohlsearch incsearch")
        surface.requestVerticalViewport(top: 320)
        let originalViewport = surface.viewportState
        let originalCursor = surface.viewPresentation.cursor_utf8_offset
        let before = try backend.recoverySnapshot()

        type("/needle", into: surface)
        try finishSearch(surface)

        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(surface.commandLine?.text, "needle")
        XCTAssertGreaterThan(surface.viewportState.top, originalViewport.top)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        surface.editorView.cancelOperation(nil)
        try finishSearch(surface)

        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, originalCursor)
        XCTAssertEqual(surface.viewportState.top, originalViewport.top, accuracy: 0.1)
        XCTAssertEqual(surface.viewportState.left, originalViewport.left, accuracy: 0.1)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
    }

    func testDetachingRetiresPendingNativeSearchPolling() throws {
        let source = String(repeating: "a\n", count: 20_000) + "needle"
        let (_, surface) = try fixture(source, settings: "set hlsearch noincsearch")
        type("/needle", into: surface)
        surface.performInput {
            _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        let pending = try XCTUnwrap(surface.session).searchWorkPending()
        XCTAssertTrue(pending)
        XCTAssertEqual(surface.isSearchPolling, pending)

        surface.detachFromCore()

        XCTAssertFalse(surface.isSearchPolling)
        XCTAssertFalse(surface.searchWorkPending)
        XCTAssertNil(surface.session)
        surface.pollSearch()
        XCTAssertFalse(surface.isSearchPolling)
    }

    func testRunLoopFinishesSearchWithoutAdditionalInput() async throws {
        let source = String(repeating: "a\n", count: 20_000) + "needle"
        let (backend, surface) = try fixture(source, settings: "set hlsearch noincsearch")
        let originalViewportTop = surface.viewportState.top
        type("/needle", into: surface)
        surface.performInput {
            _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        // The current match must paint in the input turn, before the background
        // scanner has reached this distant viewport or the run loop can poll it.
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 40_000)
        XCTAssertGreaterThan(surface.viewportState.top, originalViewportTop)
        XCTAssertTrue(try hasMatchPaint(at: 40_000, in: surface))
        XCTAssertTrue(try XCTUnwrap(surface.session).searchWorkPending())
        XCTAssertTrue(surface.isSearchPolling)
        for _ in 0..<300 where surface.isSearchPolling {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertFalse(surface.isSearchPolling)
        XCTAssertFalse(try XCTUnwrap(surface.session).searchWorkPending())
        XCTAssertTrue(try hasMatchPaint(at: 40_000, in: surface))
        XCTAssertNil(surface.commandOutput)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }
}
