import AppKit
import CViemCore
@testable import ViemAppShell
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

    private func begin(_ new: EVCoreDocumentBackend, base source: String, aligned: Bool = true, collapsed: Bool = false,
                       editable: Bool = true)
        throws -> (EVEditorSurfaceController, EVCoreDocumentBackend, EVEditorSurfaceController, any EVDiffSessionControlling)
    {
        let surface = try surface(new)
        let base = try XCTUnwrap(try new.makeDiffBase(source: Data(source.utf8)) as? EVCoreDocumentBackend)
        let baseSurface = try self.surface(base)
        let session = try new.beginDiff(base: base, baseSurface: baseSurface, surface: surface,
                                        aligned: aligned, collapsed: collapsed, editable: editable)
        return (surface, base, baseSurface, session)
    }

    func testChangedLinesShowOnBothSidesAndFollowTyping() throws {
        let new = try backend("a\nb\nc\n")
        let (surface, _, baseSurface, session) = try begin(new, base: "a\nB\nc\n")
        XCTAssertEqual(session.lineTotals.added, 1)
        XCTAssertEqual(session.lineTotals.removed, 1)
        XCTAssertEqual(try backgroundLines(surface), [1])
        XCTAssertEqual(try backgroundLines(baseSurface), [1])
        for (side, flag) in [(surface, UInt32(VIEM_LINE_NUMBER_DIFF_ADDED)), (baseSurface, UInt32(VIEM_LINE_NUMBER_DIFF_REMOVED))] {
            let view = try XCTUnwrap(side.session)
            try view.setNumber(true)
            let info = try view.lineNumberInfo()
            XCTAssertNotEqual(info.flags & flag, 0)
            XCTAssertEqual(try view.lineNumberBands(info).filter(\.isChanged).map(\.line), [2])
            let styles = try EVCoreStyleBridge.copyStyleSheet(core: side.backend.core, gutterUnderlay: flag)
            XCTAssertNotNil(styles.lineNumberDefinition(current: false)?.properties[.blockBackground]?.effective)
        }
        type("Ax", into: surface)
        escape(surface)
        XCTAssertEqual(try source(new), "ax\nb\nc\n")
        XCTAssertEqual(session.lineTotals.added, 2)
        XCTAssertEqual(session.lineTotals.removed, 2)
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

    func testTypingInsideAddedTextKeepsTheCurrentBaseFrame() throws {
        let new = try backend("one\nnew\ntwo")
        let (live, _, left, comparison) = try begin(new, base: "one\ntwo")
        defer { comparison.end() }
        type("jA", into: live)
        var exports = 0
        left.beforePresentationExport = { _ in exports += 1 }
        type("x", into: live)
        XCTAssertEqual(try source(new), "one\nnewx\ntwo")
        XCTAssertEqual(exports, 0)
    }

    func testReloadUpdatesTheComparisonAgainstItsExistingBase() throws {
        let path = "/tmp/viem-diff.md"
        let original = "first\nsecond\nthird\n"
        let new = try backend(original, type: try XCTUnwrap(EVDocument.markdownType), filename: path)
        let (live, base, left, comparison) = try begin(new, base: original)
        let baseView = try XCTUnwrap(left.session)
        let baseCore = base.core

        // Reloading reads the buffer's own format; the compared view stays Code.
        try new.read(source: Data("first\nreplacement\nthird\n".utf8), typeName: try XCTUnwrap(EVDocument.markdownType),
                     origin: .reload(path: path))
        XCTAssertEqual(live.viewFormat, .code)
        XCTAssertIdentical(left.session, baseView)
        XCTAssertEqual(base.core, baseCore)
        XCTAssertEqual(try source(base), original)
        XCTAssertEqual(comparison.lineTotals.added, 1)
        XCTAssertEqual(comparison.lineTotals.removed, 1)
        XCTAssertEqual(try backgroundLines(live), [1])
        XCTAssertEqual(try backgroundLines(left), [1])
        comparison.end()
        XCTAssertEqual(new.sourceFormat, .markdown)
    }

    func testMarkdownPreviewIsComparedAsCodeAndRestoredAfterwards() throws {
        let new = try backend("# Title\n\ntext\n", type: try XCTUnwrap(EVDocument.markdownType), filename: "/tmp/viem-diff.md")
        XCTAssertEqual(new.sourceFormat, .markdown)
        let (surface, _, _, session) = try begin(new, base: "# Title\n")
        XCTAssertEqual(surface.viewFormat, .code, "the compared view shows Code")
        XCTAssertEqual(new.sourceFormat, .markdown, "the buffer stays Markdown")
        XCTAssertEqual(try source(new), "# Title\n\ntext\n", "showing Code keeps the bytes")
        session.end()
        XCTAssertEqual(surface.viewFormat, .markdown)
        XCTAssertEqual(surface.currentDocumentMode()?.format, "markdown")
    }

    func testAnotherViewKeepsPreviewWhileTheComparedViewShowsCode() throws {
        let new = try backend("# Title\n\ntext\n", type: try XCTUnwrap(EVDocument.markdownType), filename: "/tmp/viem-diff.md")
        let other = try surface(new)
        let (compared, _, baseSurface, session) = try begin(new, base: "# Title\n")
        XCTAssertEqual(compared.viewFormat, .code)
        XCTAssertEqual(other.viewFormat, .markdown, "a view in another window keeps Preview")
        // Turning Preview off and on again elsewhere leaves the comparison literal.
        other.setMarkdownPreview(false)
        other.setMarkdownPreview(true)
        XCTAssertEqual(other.viewFormat, .markdown)
        XCTAssertEqual(compared.viewFormat, .code)
        type("ix", into: compared)
        escape(compared)
        XCTAssertEqual(try source(new), "# xTitle\n\ntext\n")
        XCTAssertEqual(session.lineTotals.added, 3, "the comparison follows the edit")
        XCTAssertEqual(session.lineTotals.removed, 1)
        XCTAssertEqual(try backgroundLines(baseSurface), [0])
        session.end()
        XCTAssertEqual(other.viewFormat, .markdown)
        XCTAssertEqual(new.sourceFormat, .markdown)
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

    func testAlignedPanesScrollContinuouslyThroughTrailingFiller() throws {
        let new = try backend("common\n" + String(repeating: "a long addition\n", count: 160))
        let (live, _, base, comparison) = try begin(new, base: "common\nshort")
        defer { comparison.end() }
        for (driver, fraction) in [(base, 0.6), (live, 0.8), (base, 1.0), (base, 0.95), (live, 0.0)] {
            let maximum = CGFloat(driver.viewportState.maximum_top)
            XCTAssertGreaterThan(maximum, 0)
            let requested = fraction == 1 ? CGFloat(Float.greatestFiniteMagnitude) : maximum * fraction
            driver.performInput { _ = try XCTUnwrap(driver.session).setViewportOrigin(left: 0, top: requested) }
            driver.refreshPresentation()
            XCTAssertEqual(base.viewportState.top, live.viewportState.top)
            XCTAssertEqual(base.viewportState.maximum_top, live.viewportState.maximum_top)
            XCTAssertEqual(CGFloat(live.viewportState.top), fraction == 1 ? CGFloat(live.viewportState.maximum_top) : requested, accuracy: 0.1)
        }
    }

    func testDeletingAnAddedLineKeepsTheLiveViewportAndMirroredBaseInPlace() throws {
        let lines = (0..<80).map { "line \($0)" }
        var edited = lines
        let added = (0..<30).map { "added \($0)" }
        edited.insert(contentsOf: added, at: 25)
        let new = try backend(edited.joined(separator: "\n"))
        let (live, _, base, comparison) = try begin(new, base: lines.joined(separator: "\n"), collapsed: true)
        defer { comparison.end() }
        type("36Gzt2j", into: live)
        func anchorY() throws -> CGFloat {
            CGFloat(try XCTUnwrap(live.layoutSnapshot?.rows.first { $0.hard_line_index == 35 }).y)
        }
        let row = try XCTUnwrap(live.layoutSnapshot?.rows.first { $0.hard_line_index == 35 })
        live.performInput {
            _ = try XCTUnwrap(live.session).setViewportOrigin(left: 0, top: CGFloat(row.y + row.line_advance / 4))
        }
        let top = live.viewportState.top
        let screenY = try anchorY() - CGFloat(top)
        type("dd", into: live)
        edited.remove(at: 37)
        XCTAssertEqual(try source(new), edited.joined(separator: "\n"))
        XCTAssertEqual(try anchorY() - CGFloat(live.viewportState.top), screenY, accuracy: 0.1)
        XCTAssertEqual(live.viewportState.top, top, accuracy: 0.1)
        XCTAssertEqual(base.viewportState.top, top, accuracy: 0.1)
        type("qadduq", into: live)
        let revision = live.documentState.document_revision
        type("@a", into: live)
        XCTAssertEqual(live.documentState.document_revision, revision)
        XCTAssertEqual(try source(new), edited.joined(separator: "\n"))
        XCTAssertTrue(try backgroundLines(live).contains(37))
        XCTAssertEqual(try anchorY() - CGFloat(live.viewportState.top), screenY, accuracy: 0.1)
        XCTAssertEqual(base.viewportState.top, live.viewportState.top, accuracy: 0.1)
    }

    func testNextChangeMovesTheCaretAndScrollsBothSidesToTheChange() throws {
        var lines = (0..<300).map { "line \($0)" }
        let base = lines.joined(separator: "\n")
        lines[200] = "changed"
        let new = try backend(lines.joined(separator: "\n"))
        let (surface, _, baseSurface, session) = try begin(new, base: base)
        session.navigateChange(next: true)
        let offset = try XCTUnwrap(source(new).range(of: "changed")).lowerBound.utf16Offset(in: try source(new))
        XCTAssertEqual(Int(surface.viewPresentation.cursor_utf8_offset), offset, "the caret is on the changed line")
        XCTAssertGreaterThan(surface.viewportState.top, 0)
        let shown = try topLine(surface)
        XCTAssertLessThan(shown, 200)
        XCTAssertEqual(try topLine(baseSurface), shown, "the base lines up with the new side")
        // The only change wraps to itself without moving the view.
        let top = surface.viewportState.top
        session.navigateChange(next: false)
        XCTAssertEqual(surface.viewportState.top, top)
    }

    func testScrollbarMarksShowChangesInTheLineColorsUntilTheComparisonEnds() throws {
        let lines = (0..<300).map { "line \($0)" }
        var changed = lines
        changed.insert("added", at: 150)
        let new = try backend(changed.joined(separator: "\n"))
        let (surface, _, baseSurface, session) = try begin(new, base: lines.joined(separator: "\n"))
        let marks = surface.editorView.documentScrollbars.changeMarks
        XCTAssertEqual(marks.count, 1)
        let mark = try XCTUnwrap(marks.first)
        let extent = surface.editorView.documentScrollbars.changeMarkExtent
        XCTAssertEqual(mark.top / extent, 150.0 / 300.0, accuracy: 0.02, "the mark sits where the line is")
        XCTAssertGreaterThan(mark.color.alphaComponent, 0)
        XCTAssertGreaterThan(mark.color.greenComponent, mark.color.redComponent, "added lines are green")
        let baseMark = try XCTUnwrap(baseSurface.editorView.documentScrollbars.changeMarks.first)
        XCTAssertEqual(baseMark.color, mark.color, "the base marks the added line at its filler")
        session.end()
        XCTAssertTrue(surface.editorView.documentScrollbars.changeMarks.isEmpty)
    }

    func testChangeIconsAreTemplateImagesOfTheirOwnShape() {
        let next = EVDiffToolbar.changeIcon(next: true)
        let previous = EVDiffToolbar.changeIcon(next: false)
        XCTAssertTrue(next.isTemplate)
        XCTAssertNotEqual(next.tiffRepresentation, previous.tiffRepresentation)
    }

    func testAlignSidesIsAnIconToggleLikeWindows() {
        let toolbar = EVDiffToolbar()
        let align = toolbar.alignButton
        XCTAssertEqual(align.title, "")
        XCTAssertTrue(align.image?.isTemplate ?? false)
        XCTAssertEqual(align.image?.size, NSSize(width: 18, height: 16))
        XCTAssertEqual(align.toolTip, "Align Sides")
        XCTAssertEqual(align.accessibilityLabel(), "Align Sides")
        align.state = .off
        align.performClick(nil)
        XCTAssertEqual(align.state, .on, "a click toggles the button's own state")
    }

    /// Hard lines that have rows.
    private func shownLines(_ surface: EVEditorSurfaceController) throws -> Set<UInt64> {
        Set(try XCTUnwrap(surface.layoutSnapshot).rows.map(\.hard_line_index))
    }

    /// Hard lines whose rows draw a collapsed-context divider band.
    private func dividerLines(_ surface: EVEditorSurfaceController) throws -> Set<UInt64> {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let rows = Dictionary(uniqueKeysWithValues: snapshot.rows.map { ($0.row_index, $0.hard_line_index) })
        let band = UInt32(VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT | VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND)
        return Set(snapshot.decorations.filter { $0.flags & band == band }.compactMap { rows[$0.row_index] })
    }

    /// Base lines 0..<40; the new side changes lines 10 and 30, so lines
    /// 0..<7, 14..<27 and 34..<40 collapse.
    private func beginCollapsed() throws
        -> (EVEditorSurfaceController, EVEditorSurfaceController, any EVDiffSessionControlling, EVCoreDocumentBackend)
    {
        var lines = (0..<40).map { "line \($0)" }
        let base = lines.joined(separator: "\n")
        lines[10] = "changed 10"
        lines[30] = "changed 30"
        let new = try backend(lines.joined(separator: "\n"))
        let (surface, _, baseSurface, session) = try begin(new, base: base, collapsed: true)
        return (surface, baseSurface, session, new)
    }

    private func send(_ text: String, to surface: EVEditorSurfaceController) {
        let session = surface.session
        surface.performInput {
            for scalar in text.unicodeScalars {
                _ = try XCTUnwrap(session).sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: scalar.value)
            }
        }
    }

    private func enter(_ surface: EVEditorSurfaceController) {
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
    }

    func testCollapsedContextHidesTheSameUnchangedLinesOnBothSidesUntilItsDividerIsClicked() throws {
        let (surface, baseSurface, session, _) = try beginCollapsed()
        let hidden = Set<UInt64>((0..<7).map(UInt64.init) + (14..<27).map(UInt64.init))
        for side in [surface, baseSurface] {
            let shown = try shownLines(side)
            XCTAssertTrue(shown.isDisjoint(with: hidden), "hidden lines have no rows")
            XCTAssertTrue(shown.isSuperset(of: [7, 13, 27]))
            XCTAssertTrue(try dividerLines(side).isSuperset(of: [7, 13]))
        }
        XCTAssertEqual(surface.layoutSnapshot?.rows.first { $0.hard_line_index == 27 }?.y,
                       baseSurface.layoutSnapshot?.rows.first { $0.hard_line_index == 27 }?.y, "the sides stay level")
        // A click on the divider after line 13 opens 14..<27 on both sides.
        XCTAssertFalse(surface.diffOpenCollapsedContext?(9) ?? true, "no divider borders line 9")
        XCTAssertTrue(surface.diffOpenCollapsedContext?(13) ?? false)
        XCTAssertTrue(try shownLines(surface).contains(20))
        XCTAssertTrue(try shownLines(baseSurface).contains(20))
        XCTAssertFalse(try dividerLines(baseSurface).contains(13))
        session.setCollapsed(false)
        XCTAssertTrue(try shownLines(surface).contains(0))
        XCTAssertTrue(try dividerLines(surface).isEmpty)
    }

    func testACaretMovingOntoCollapsedContextOpensItOnBothSides() throws {
        let (surface, baseSurface, session, new) = try beginCollapsed()
        defer { withExtendedLifetime(session) {} }
        let text = try source(new)
        let offset = { (needle: String) in try XCTUnwrap(text.range(of: needle)).lowerBound.utf16Offset(in: text) }
        XCTAssertEqual(Int(surface.viewPresentation.cursor_utf8_offset), try offset("line 7"),
                       "the starting caret moves beside the collapsed first lines")
        send(":14", to: surface)
        enter(surface)
        send("j", to: surface)
        XCTAssertEqual(Int(surface.viewPresentation.cursor_utf8_offset), try offset("line 14"))
        XCTAssertTrue(try shownLines(surface).contains(14))
        XCTAssertTrue(try shownLines(baseSurface).contains(14), "the base opens in the same turn")
    }

    func testCollapseContextIsAnIconToggle() {
        let collapse = EVDiffToolbar().collapseButton
        XCTAssertEqual(collapse.title, "")
        XCTAssertTrue(collapse.image?.isTemplate ?? false)
        XCTAssertEqual(collapse.toolTip, "Collapse Context")
        collapse.state = .off
        collapse.performClick(nil)
        XCTAssertEqual(collapse.state, .on)
    }

    func testCodeRemembersOneWordWrapForNewViews() throws {
        let configuration = try configuration()
        let code = EVCoreDocumentBackend(configuration: configuration)
        try code.read(source: Data("let x = 1\n".utf8), typeName: EVDocument.codeType,
                      origin: .file(path: "/tmp/viem-wrap.rs", automaticCode: true))
        let first = try surface(code)
        XCTAssertFalse(first.wrapEnabled, "Code starts unwrapped until a choice is remembered")
        first.perform(menuCommand: .wordWrap, sender: nil)
        XCTAssertTrue(first.wrapEnabled)
        XCTAssertTrue(configuration.codeWordWrap)
        XCTAssertTrue(try surface(code).wrapEnabled, "a new Code view uses the last choice")
        try configuration.setCodeWordWrap(false)
        let text = EVCoreDocumentBackend(configuration: configuration)
        try text.read(source: Data("words\n".utf8), typeName: EVDocument.plainTextType, origin: .untitled)
        XCTAssertTrue(try surface(text).wrapEnabled, "Text keeps its own wrapping")
    }

    func testBothDiffSidesShareCodesWordWrapAndProseResumesAfterwards() throws {
        let configuration = try configuration()
        let new = EVCoreDocumentBackend(configuration: configuration)
        try new.read(source: Data("# Title\n\ntext\n".utf8), typeName: try XCTUnwrap(EVDocument.markdownType),
                     origin: .file(path: "/tmp/viem-wrap.md", automaticCode: true))
        let surface = try surface(new)
        XCTAssertTrue(surface.wrapEnabled, "Markdown wraps")
        let base = try XCTUnwrap(try new.makeDiffBase(source: Data("# Title\n".utf8)) as? EVCoreDocumentBackend)
        let baseSurface = try self.surface(base)
        let session = try new.beginDiff(base: base, baseSurface: baseSurface, surface: surface, aligned: true,
                                        collapsed: false, editable: true)
        XCTAssertFalse(surface.wrapEnabled, "the compared side uses Code's choice, off by default")
        XCTAssertFalse(baseSurface.wrapEnabled)
        baseSurface.perform(menuCommand: .wordWrap, sender: nil)
        XCTAssertTrue(baseSurface.wrapEnabled)
        XCTAssertTrue(surface.wrapEnabled, "the other side follows")
        XCTAssertTrue(configuration.codeWordWrap, "diff mode and Code share the choice")
        surface.perform(menuCommand: .wordWrap, sender: nil)
        XCTAssertFalse(baseSurface.wrapEnabled)
        XCTAssertFalse(configuration.codeWordWrap)
        session.end()
        XCTAssertEqual(new.sourceFormat, .markdown)
        XCTAssertTrue(surface.wrapEnabled, "Markdown resumes its own wrapping")
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
