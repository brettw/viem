import AppKit
import CEvimCore
import EvimCoreTextProvider
import XCTest

@testable import EvimEditor

final class EVCoreFrontendIntegrationTests: XCTestCase {
    @MainActor
    func testEditingLayoutRenderingAndNativeHistoryRoundTrip() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("world".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)

        _ = try session.sendText("i")
        _ = try session.sendText("Hello ")
        surface.refreshPresentation()
        XCTAssertEqual(surface.formattedText, "Hello world")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(EVIM_MODE_INSERT))

        let layout = try session.layoutExport()
        XCTAssertFalse(layout.rows.isEmpty)
        XCTAssertFalse(layout.clusters.isEmpty)
        XCTAssertFalse(layout.carets.isEmpty)
        XCTAssertEqual(layout.info.identity.document_revision, try backend.revision())
        XCTAssertTrue(layout.clusters.allSatisfy { cluster in
            guard cluster.flags & UInt32(EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0 else {
                return false
            }
            return session.provider.renderRegistry.contains(
                identifier: cluster.render_run.identifier,
                metricsGeneration: cluster.render_run.metrics_generation
            )
        })

        _ = try session.undo()
        surface.refreshPresentation()
        XCTAssertEqual(surface.formattedText, "world")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(EVIM_MODE_NORMAL))

        _ = try session.redo()
        surface.refreshPresentation()
        XCTAssertEqual(surface.formattedText, "Hello world")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(EVIM_MODE_NORMAL))
    }

    @MainActor
    func testPointerHitTestAndShiftExtensionRemainCoreOwned() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("alpha beta".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        var layout = try session.layoutExport()

        let first = try session.hitTest(CGPoint(x: 0, y: 0), in: layout.info)
        _ = try session.placeCursor(first, extendSelection: false)
        layout = try session.layoutExport()
        let last = try session.hitTest(CGPoint(x: 10_000, y: 0), in: layout.info)
        _ = try session.placeCursor(last, extendSelection: true)
        surface.refreshPresentation()

        XCTAssertEqual(surface.viewPresentation.mode, UInt32(EVIM_MODE_VISUAL_CHARACTER))
        XCTAssertNotEqual(
            surface.viewPresentation.flags & UInt32(EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR),
            0
        )
        XCTAssertNotNil(surface.selectedUTF8Range())
    }

    @MainActor
    func testIMECommitIsAtomicAndCancelLeavesSourceUntouched() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)

        _ = try session.sendText("i")
        _ = try session.beginComposition(replacing: 0 ..< 0)
        _ = try session.updateComposition("e\u{301}", selected: 3 ..< 3)
        XCTAssertEqual(try backend.formattedText(), "")
        _ = try session.commitComposition("é")
        XCTAssertEqual(try backend.formattedText(), "é")

        _ = try session.beginComposition(replacing: 2 ..< 2)
        _ = try session.updateComposition("漢字", selected: 6 ..< 6)
        _ = try session.cancelComposition()
        XCTAssertEqual(try backend.formattedText(), "é")

        _ = try session.undo()
        XCTAssertEqual(try backend.formattedText(), "")
    }

    @MainActor
    func testFrontendUsesSFProFourteenPointDefault() {
        XCTAssertEqual(CoreTextMeasurementProvider.defaultFontFamily, "SF Pro")
        XCTAssertEqual(CoreTextMeasurementProvider.defaultFontSize, 14)
    }

    @MainActor
    func testRetainedEditorViewBecomesInertAfterSurfaceControllerRelease() async throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("transient window".utf8), typeName: "public.plain-text")

        var surface: EVEditorSurfaceController? = try XCTUnwrap(
            backend.makeEditorSurface() as? EVEditorSurfaceController
        )
        surface?.loadViewIfNeeded()
        surface?.view.frame = NSRect(x: 0, y: 0, width: 420, height: 220)
        surface?.viewDidLayout()
        let retainedView = try XCTUnwrap(surface?.editorView)
        weak var releasedSurface: EVEditorSurfaceController?
        releasedSurface = surface

        surface = nil
        XCTAssertNil(releasedSurface, "the retained NSView must not keep its controller alive")
        XCTAssertNil(retainedView.surface)

        // Match AppKit's close path: the window/controller can disappear in
        // one event-loop turn and native code may ask the retained view for
        // presentation and text-client state in a later turn.
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async { continuation.resume() }
        }

        XCTAssertTrue(retainedView.isOpaque)
        XCTAssertNil(retainedView.accessibilityValue())
        XCTAssertEqual(retainedView.accessibilityNumberOfCharacters(), 0)
        XCTAssertEqual(retainedView.selectedRange(), NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(retainedView.characterIndex(for: .zero), NSNotFound)
        XCTAssertEqual(retainedView.viewportOrigin, .zero)

        // These are representative late AppKit presentation callbacks. They
        // must be harmless even though their surface no longer exists.
        retainedView.applyPresentation()
        let image = NSImage(size: retainedView.bounds.size)
        image.lockFocus()
        retainedView.draw(retainedView.bounds)
        image.unlockFocus()
    }

    @MainActor
    func testCanvasInsetsAreExcludedFromCoreViewportDimensions() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("one line".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()

        let viewSize = CGSize(width: 420, height: 260)
        surface.view.frame = NSRect(origin: .zero, size: viewSize)
        surface.viewDidLayout()

        let expected = EVEditorView.layoutViewportSize(for: viewSize)
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(CGFloat(layout.info.viewport_width), expected.width, accuracy: 0.01)
        XCTAssertEqual(CGFloat(layout.info.viewport_height), expected.height, accuracy: 0.01)
        XCTAssertEqual(expected.width, 360)
        XCTAssertEqual(expected.height, 212)
    }

    @MainActor
    func testKeyboardViewportOriginIsReflectedByViewCoordinates() throws {
        let source = (0..<120).map { "line \($0)" }.joined(separator: "\n")
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 130)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)

        surface.performInput { _ = try session.sendText("G") }

        XCTAssertGreaterThan(surface.viewportState.top, 0)
        XCTAssertEqual(surface.editorView.viewportOrigin.y, CGFloat(surface.viewportState.top), accuracy: 0.01)
        let viewportCorner = surface.editorView.viewPoint(
            fromLayoutPoint: surface.editorView.viewportOrigin
        )
        XCTAssertEqual(viewportCorner.x, EVEditorView.canvasInsets.left, accuracy: 0.01)
        XCTAssertEqual(viewportCorner.y, EVEditorView.canvasInsets.top, accuracy: 0.01)
        let roundTrip = surface.editorView.layoutPoint(fromViewPoint: viewportCorner)
        XCTAssertEqual(roundTrip.x, surface.editorView.viewportOrigin.x, accuracy: 0.01)
        XCTAssertEqual(roundTrip.y, surface.editorView.viewportOrigin.y, accuracy: 0.01)
    }

    @MainActor
    func testHorizontalViewportOriginRoundTripsThroughCore() throws {
        let backend = EVCoreDocumentBackend()
        let source = String(repeating: "proportional text ", count: 80)
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 260, height: 160)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.setWrap(false) }

        let maximum = CGFloat(surface.viewportState.maximum_left)
        XCTAssertGreaterThan(maximum, 0)
        let requested = min(75, maximum)
        surface.performInput { _ = try session.setViewportOrigin(left: requested) }

        XCTAssertEqual(CGFloat(surface.viewportState.left), requested, accuracy: 0.01)
        XCTAssertEqual(surface.editorView.viewportOrigin.x, requested, accuracy: 0.01)
    }

    @MainActor
    func testIdentityBoundVerticalViewportUpdatesBoundedRegionalCoverage() throws {
        let backend = EVCoreDocumentBackend()
        let source = (0..<2_000).map { "regional line \($0)" }.joined(separator: "\n")
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 120)
        surface.viewDidLayout()
        let initial = try XCTUnwrap(surface.layoutSnapshot)
        let initialState = surface.viewportState

        surface.requestVerticalViewport(top: 14_000)

        let moved = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertGreaterThan(surface.viewportState.top, 10_000)
        XCTAssertGreaterThan(
            moved.info.coverage_hard_line_start,
            initial.info.coverage_hard_line_start
        )
        XCTAssertLessThan(
            moved.info.coverage_hard_line_end - moved.info.coverage_hard_line_start,
            100
        )
        XCTAssertLessThanOrEqual(moved.info.coverage_y_start, surface.viewportState.top)
        XCTAssertGreaterThanOrEqual(
            moved.info.coverage_y_end,
            surface.viewportState.top + moved.info.viewport_height
        )
        XCTAssertNotEqual(surface.viewportState.layout_revision, initialState.layout_revision)
        XCTAssertEqual(
            moved.info.identity.layout_revision,
            surface.viewportState.layout_revision,
            "the surface must consume the identity returned by the vertical request"
        )
    }

    @MainActor
    func testSharedDocumentChangeRefreshesEverySurfaceExactlyOnce() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("world".utf8), typeName: "public.plain-text")
        let first = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        first.loadViewIfNeeded()
        second.loadViewIfNeeded()
        let session = try XCTUnwrap(first.session)
        let firstRefreshes = first.presentationRefreshCount
        let secondRefreshes = second.presentationRefreshCount
        var sourceChangeNotifications = 0
        backend.sourceDidChange = { sourceChangeNotifications += 1 }

        first.performInput {
            _ = try session.sendText("i")
            _ = try session.sendText("Hello ")
        }

        XCTAssertEqual(first.formattedText, "Hello world")
        XCTAssertEqual(second.formattedText, "Hello world")
        XCTAssertEqual(first.presentationRefreshCount, firstRefreshes + 1)
        XCTAssertEqual(second.presentationRefreshCount, secondRefreshes + 1)
        XCTAssertEqual(sourceChangeNotifications, 1)
    }

    @MainActor
    func testSharedCommitClearsInvalidatedSiblingMarkedText() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
        let writer = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let composing = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        writer.loadViewIfNeeded()
        composing.loadViewIfNeeded()
        let writerSession = try XCTUnwrap(writer.session)
        let composingSession = try XCTUnwrap(composing.session)

        composing.performInput { _ = try composingSession.sendText("i") }
        composing.editorView.setMarkedText(
            "é",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertTrue(composing.editorView.hasMarkedText())
        XCTAssertTrue(composingSession.hasActiveComposition)

        writer.performInput {
            _ = try writerSession.sendText("i")
            _ = try writerSession.sendText("X")
        }

        XCTAssertFalse(composing.editorView.hasMarkedText())
        XCTAssertFalse(composingSession.hasActiveComposition)
        XCTAssertEqual(composing.formattedText, "Xbase")
    }
}
