import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

/// Diff mode's editor side: the protected base buffer and the comparison
/// session's presentation, mode switching and scroll mirroring.
@MainActor
final class EVDiffModeTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func configuration() throws -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-diff-mode-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func backend(_ source: String, type: String = EVDocument.codeType, filename: String = "/tmp/viem-diff.rs") throws -> EVCoreDocumentBackend {
        let backend = EVCoreDocumentBackend(configuration: try configuration())
        try backend.read(source: Data(source.utf8), typeName: type, origin: .file(path: filename, automaticCode: true))
        return backend
    }

    private func surface(_ backend: any EVDocumentBackend, height: CGFloat = 260) throws -> EVEditorSurfaceController {
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 480, height: height)
        surface.viewDidLayout()
        return surface
    }

    private func type(_ text: String, into surface: EVEditorSurfaceController) {
        surface.editorView.insertText(text, replacementRange: noReplacement)
    }

    private func escape(_ surface: EVEditorSurfaceController) {
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
    }

    /// Hard lines drawn with a block background (a diff line style).
    private func backgroundLines(_ surface: EVEditorSurfaceController) throws -> Set<UInt64> {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let rows = Dictionary(uniqueKeysWithValues: snapshot.rows.map { ($0.row_index, $0.hard_line_index) })
        return Set(snapshot.decorations
            .filter { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0 }
            .compactMap { rows[$0.row_index] })
    }

    private func source(_ backend: EVCoreDocumentBackend) throws -> String {
        String(decoding: try backend.recoverySnapshot().source, as: UTF8.self)
    }

    private func begin(_ new: EVCoreDocumentBackend, base source: String, aligned: Bool = true, editable: Bool = true)
        throws -> (EVEditorSurfaceController, EVCoreDocumentBackend, EVEditorSurfaceController, any EVDiffSessionControlling)
    {
        let surface = try surface(new)
        let base = try XCTUnwrap(try new.makeDiffBase(source: Data(source.utf8)) as? EVCoreDocumentBackend)
        let baseSurface = try self.surface(base)
        let session = try new.beginDiff(base: base, baseSurface: baseSurface, surface: surface,
                                        aligned: aligned, editable: editable)
        return (surface, base, baseSurface, session)
    }

    func testChangedLinesShowOnBothSidesAndFollowTyping() throws {
        let new = try backend("a\nb\nc\n")
        let (surface, _, baseSurface, session) = try begin(new, base: "a\nB\nc\n")
        XCTAssertEqual(try backgroundLines(surface), [1])
        XCTAssertEqual(try backgroundLines(baseSurface), [1])
        type("Ax", into: surface)
        escape(surface)
        XCTAssertEqual(try source(new), "ax\nb\nc\n")
        XCTAssertEqual(try backgroundLines(surface), [0, 1])
        XCTAssertEqual(try backgroundLines(baseSurface), [0, 1])
        session.end()
        XCTAssertEqual(try backgroundLines(surface), [])
        XCTAssertEqual(try source(new), "ax\nb\nc\n", "ending the comparison never edits the document")
    }

    func testTheBaseBufferRejectsEditsAndIsReadOnly() throws {
        let new = try backend("one\n")
        let (_, base, baseSurface, _) = try begin(new, base: "base\n")
        type("ix", into: baseSurface)
        escape(baseSurface)
        type("dd", into: baseSurface)
        XCTAssertEqual(try source(base), "base\n")
        XCTAssertTrue(base.persistenceState.isReadOnly)
        XCTAssertFalse(base.persistenceState.isDirty)
    }

    func testMarkdownPreviewIsComparedAsCodeAndRestoredAfterwards() throws {
        let new = try backend("# Title\n\ntext\n", type: try XCTUnwrap(EVDocument.markdownType), filename: "/tmp/viem-diff.md")
        XCTAssertEqual(new.sourceFormat, .markdown)
        let (surface, _, _, session) = try begin(new, base: "# Title\n")
        XCTAssertEqual(new.sourceFormat, .code)
        XCTAssertEqual(try source(new), "# Title\n\ntext\n", "switching to Code keeps the bytes")
        session.end()
        XCTAssertEqual(new.sourceFormat, .markdown)
        XCTAssertEqual(surface.currentDocumentMode()?.format, "markdown")
    }

    func testTheFormatIsRestoredThroughAnotherViewAfterTheComparedPaneCloses() throws {
        let new = try backend("# Title\n\ntext\n", type: try XCTUnwrap(EVDocument.markdownType), filename: "/tmp/viem-diff.md")
        let other = try surface(new)
        weak var compared: EVEditorSurfaceController?
        var session: (any EVDiffSessionControlling)?
        try autoreleasepool {
            let begun = try begin(new, base: "# Title\n")
            compared = begun.0
            session = begun.3
        }
        XCTAssertNil(compared, "the compared pane has closed")
        XCTAssertEqual(new.sourceFormat, .code)
        try XCTUnwrap(session).end()
        XCTAssertEqual(new.sourceFormat, .markdown)
        XCTAssertEqual(other.currentDocumentMode()?.format, "markdown")
    }

    func testADeletedFilesBlankSideIsNotEditableUntilTheComparisonEnds() throws {
        let new = try backend("")
        let (surface, _, _, session) = try begin(new, base: "gone\n", editable: false)
        type("ix", into: surface)
        escape(surface)
        XCTAssertEqual(try source(new), "")
        session.end()
        type("ix", into: surface)
        escape(surface)
        XCTAssertEqual(try source(new), "x")
    }

    /// The hard line drawn at the top of the viewport.
    private func topLine(_ surface: EVEditorSurfaceController) throws -> UInt64 {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let top = surface.viewportState.top
        return try XCTUnwrap(snapshot.rows.first { $0.y + $0.line_advance > top }).hard_line_index
    }

    func testAlignedSidesScrollTogether() throws {
        let lines = (0..<400).map { "line \($0)" }
        let new = try backend(lines.joined(separator: "\n"))
        var baseLines = lines
        baseLines.insert(contentsOf: ["extra 1", "extra 2"], at: 10)
        let (surface, _, baseSurface, session) = try begin(new, base: baseLines.joined(separator: "\n"))
        surface.performInput { _ = try XCTUnwrap(surface.session).setViewportOrigin(left: 0, top: 900) }
        surface.refreshPresentation()
        XCTAssertGreaterThan(surface.viewportState.top, 0)
        // Past the inserted lines, new line n corresponds to base line n + 2.
        let newTop = try topLine(surface)
        XCTAssertGreaterThan(newTop, 12)
        XCTAssertEqual(try topLine(baseSurface), newTop + 2)
        session.setAligned(false)
        let baseTop = baseSurface.viewportState.top
        surface.performInput { _ = try XCTUnwrap(surface.session).setViewportOrigin(left: 0, top: 300) }
        surface.refreshPresentation()
        XCTAssertEqual(baseSurface.viewportState.top, baseTop, "unaligned sides scroll independently")
    }

    func testLastSavedSourceComesFromMemory() throws {
        let new = try backend("saved\n")
        XCTAssertEqual(new.diffSavedSource(), Data("saved\n".utf8))
        let surface = try surface(new)
        type("Ax", into: surface)
        escape(surface)
        XCTAssertEqual(new.diffSavedSource(), Data("saved\n".utf8), "an edit is not a save")
    }
}
