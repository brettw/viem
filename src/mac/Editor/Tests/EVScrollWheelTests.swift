import AppKit
import CEvimCore
import XCTest

@testable import EvimEditor

@MainActor
final class EVScrollWheelTests: XCTestCase {
    func testVerticalWheelUsesAppKitDirectionAndUnitsAndClampsAtBothEnds() throws {
        let lineCount = 240
        let source = (0..<lineCount).map { "vertical line \($0)" }.joined(separator: "\n")
        let (_, surface) = try makeSurface(
            source: source,
            size: NSSize(width: 320, height: 128)
        )
        let view = surface.editorView
        let initial = try XCTUnwrap(surface.layoutSnapshot)
        let firstRow = try XCTUnwrap(initial.rows.first)
        let discreteRowDistance = max(CGFloat(firstRow.line_advance), 1)

        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)

        view.scrollWheel(with: WheelEvent(deltaY: -1, precise: false))
        XCTAssertEqual(
            CGFloat(surface.viewportState.top),
            discreteRowDistance,
            accuracy: 0.05,
            "a non-precise wheel unit represents one visual-row distance"
        )

        view.scrollWheel(with: WheelEvent(deltaY: 1, precise: false))
        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)

        view.scrollWheel(with: WheelEvent(deltaY: -23.5, precise: true))
        XCTAssertEqual(
            surface.viewportState.top,
            23.5,
            accuracy: 0.05,
            "negative AppKit scrolling deltas advance the y-down viewport"
        )

        view.scrollWheel(with: WheelEvent(deltaY: 10_000, precise: true))
        XCTAssertEqual(surface.viewportState.top, 0, accuracy: 0.01)

        view.scrollWheel(with: WheelEvent(deltaY: -1_000_000, precise: true))
        let bottom = try XCTUnwrap(surface.layoutSnapshot)
        let expectedBottom = max(
            CGFloat(bottom.info.coverage_y_start),
            CGFloat(bottom.info.coverage_y_end - bottom.info.viewport_height)
        )
        XCTAssertEqual(bottom.info.coverage_hard_line_end, UInt64(lineCount))
        XCTAssertEqual(CGFloat(surface.viewportState.top), expectedBottom, accuracy: 0.1)

        let clampedBottom = surface.viewportState.top
        view.scrollWheel(with: WheelEvent(deltaY: -10_000, precise: true))
        XCTAssertEqual(surface.viewportState.top, clampedBottom, accuracy: 0.01)
    }

    func testHorizontalWheelUsesAppKitDirectionAndUnitsClampsAndHonorsWrap() throws {
        let source = (0..<20)
            .map { _ in String(repeating: "W", count: 240) }
            .joined(separator: "\n")
        let (_, surface) = try makeSurface(
            source: source,
            size: NSSize(width: 240, height: 150)
        )
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.setWrap(false) }

        let unwrapped = try XCTUnwrap(surface.layoutSnapshot)
        let firstRow = try XCTUnwrap(unwrapped.rows.first)
        let discreteRowDistance = max(CGFloat(firstRow.line_advance), 1)
        let maximumLeft = surface.viewportState.flags
            & UInt32(EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT) != 0
            ? CGFloat(surface.viewportState.maximum_left)
            : max(
                0,
                CGFloat(unwrapped.info.content_width - unwrapped.info.viewport_width)
            )
        XCTAssertGreaterThan(maximumLeft, discreteRowDistance)

        surface.editorView.scrollWheel(with: WheelEvent(deltaX: -1, precise: false))
        XCTAssertEqual(
            CGFloat(surface.viewportState.left),
            discreteRowDistance,
            accuracy: 0.05,
            "horizontal wheel units receive the same readable line-step conversion"
        )

        surface.editorView.scrollWheel(with: WheelEvent(deltaX: -17.25, precise: true))
        XCTAssertEqual(
            CGFloat(surface.viewportState.left),
            discreteRowDistance + 17.25,
            accuracy: 0.05,
            "negative AppKit scrolling deltas advance the y-down/rightward viewport"
        )

        surface.editorView.scrollWheel(with: WheelEvent(deltaX: -1_000_000, precise: true))
        XCTAssertEqual(CGFloat(surface.viewportState.left), maximumLeft, accuracy: 0.1)

        surface.editorView.scrollWheel(with: WheelEvent(deltaX: 1_000_000, precise: true))
        XCTAssertEqual(surface.viewportState.left, 0, accuracy: 0.01)

        surface.performInput { _ = try session.setViewportOrigin(left: 40) }
        XCTAssertGreaterThan(surface.viewportState.left, 0)
        surface.performInput { _ = try session.setWrap(true) }
        XCTAssertNotEqual(surface.viewportState.flags & UInt32(EVIM_VIEWPORT_STATE_WRAP), 0)
        XCTAssertEqual(surface.viewportState.maximum_left, 0, accuracy: 0.01)
        XCTAssertEqual(surface.viewportState.left, 0, accuracy: 0.01)

        surface.editorView.scrollWheel(with: WheelEvent(deltaX: -80, precise: true))
        XCTAssertEqual(
            surface.viewportState.left,
            0,
            accuracy: 0.01,
            "wrapped views do not acquire a horizontal viewport offset"
        )
    }

    private func makeSurface(
        source: String,
        size: NSSize
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(
            backend.makeEditorSurface() as? EVEditorSurfaceController
        )
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(origin: .zero, size: size)
        surface.viewDidLayout()
        return (backend, surface)
    }
}

private final class WheelEvent: NSEvent {
    private let horizontalDelta: CGFloat
    private let verticalDelta: CGFloat
    private let precise: Bool

    init(deltaX: CGFloat = 0, deltaY: CGFloat = 0, precise: Bool) {
        horizontalDelta = deltaX
        verticalDelta = deltaY
        self.precise = precise
        super.init()
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    override var type: NSEvent.EventType { .scrollWheel }
    override var scrollingDeltaX: CGFloat { horizontalDelta }
    override var scrollingDeltaY: CGFloat { verticalDelta }
    override var isDirectionInvertedFromDevice: Bool { true }
    override var hasPreciseScrollingDeltas: Bool { precise }
    override var phase: NSEvent.Phase { [] }
    override var momentumPhase: NSEvent.Phase { [] }
}
