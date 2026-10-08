import AppKit
import CoreText
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVImageScrollingTests: XCTestCase {
    func testScrollingThroughDemoUsesTextSizedWheelStepsOnLoadedImages() throws {
        var repository = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { repository.deleteLastPathComponent() }
        let demo = repository.appendingPathComponent("docs/markdown_demo.md")
        let source = try Data(contentsOf: demo)
        let host = ImageScrollDocumentHost(url: demo)
        let backend = EVCoreDocumentBackend()
        try backend.read(source: source, typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.documentHostEffectHandler = host
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 650, height: 500)
        surface.viewDidLayout()
        surface.refreshPresentation()
        defer { surface.imagePopover.close() }

        // Keep the caret at the start, as when opening the demo and scrolling.
        // In particular, do not pre-layout the whole document or move to its image.
        var loadedImage: ViemPositionedClusterV1?
        for _ in 0..<500 {
            if let image = surface.layoutSnapshot?.clusters.first(where: {
                surface.editorView.isImageCluster($0) && $0.typographic_bounds.height > 100
            }) {
                loadedImage = image
                break
            }
            surface.editorView.scrollWheel(with: ImageWheelEvent(deltaY: -60, precise: true))
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
        }
        let image = try XCTUnwrap(loadedImage, "the demo's relative local image should load while scrolling")
        let session = try XCTUnwrap(surface.session)
        let font = try XCTUnwrap(session.provider.renderRegistry.resolvedFont(
            identifier: image.render_run.identifier,
            metricsGeneration: image.render_run.metrics_generation))
        let textLineHeight = max(CTFontGetAscent(font) + CTFontGetDescent(font) + CTFontGetLeading(font), 1)
        surface.requestVerticalViewport(top: CGFloat(image.typographic_bounds.y) + 10)
        let before = surface.viewportState.top
        surface.editorView.scrollWheel(with: ImageWheelEvent(deltaY: -1, precise: false))
        XCTAssertEqual(CGFloat(surface.viewportState.top - before), textLineHeight, accuracy: 0.1,
                       "a wheel notch over an image must not jump by the image's height")
        surface.editorView.scrollWheel(with: ImageWheelEvent(deltaY: 1, precise: false))
        XCTAssertEqual(surface.viewportState.top, before, accuracy: 0.1)

        // Trackpads retain their point deltas, and idle image callbacks must not
        // reveal the offscreen caret or move an already measured image.
        surface.editorView.scrollWheel(with: ImageWheelEvent(deltaY: -23.5, precise: true))
        let after = surface.viewportState.top
        XCTAssertEqual(after - before, 23.5, accuracy: 0.1)
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.15))
        XCTAssertEqual(surface.viewportState.top, after, accuracy: 0.1)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        withExtendedLifetime(host) {}
    }
}

@MainActor
private final class ImageScrollDocumentHost: EVDocumentHostEffectHandling {
    let url: URL
    init(url: URL) { self.url = url }
    func documentURL(for surface: any EVEditorSurface) -> URL? { url }
    func perform(documentHostRequests: [EVDocumentHostRequest],
                 completion: @escaping @MainActor (Result<String?, Error>) -> Void) {
        XCTFail("scrolling must not issue document host effects")
        completion(.failure(EVDocumentHostError.unsupportedRequest))
    }
}

private final class ImageWheelEvent: NSEvent {
    let verticalDelta: CGFloat
    let precise: Bool
    init(deltaY: CGFloat, precise: Bool) {
        verticalDelta = deltaY
        self.precise = precise
        super.init()
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    override var type: NSEvent.EventType { .scrollWheel }
    override var scrollingDeltaX: CGFloat { 0 }
    override var scrollingDeltaY: CGFloat { verticalDelta }
    override var hasPreciseScrollingDeltas: Bool { precise }
    override var phase: NSEvent.Phase { [] }
    override var momentumPhase: NSEvent.Phase { [] }
}
