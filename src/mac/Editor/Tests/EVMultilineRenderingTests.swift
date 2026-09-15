import AppKit
import CViemCore
import XCTest

@testable import ViemEditor
@testable import ViemAppShell

final class EVMultilineRenderingTests: XCTestCase {
    @MainActor
    func testNativeDocumentWindowLaysOutEveryInitialHardLine() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(
            source: Data("FIRST ROW\nSECOND ROW\nTHIRD ROW".utf8),
            typeName: "public.plain-text"
        )
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let document = EVDocument(editorBackend: backend)
        let windowController = EVDocumentWindowController(
            document: document,
            editorSurface: surface,
            placement: EVDocumentWindowPlacement(loadFrame: { nil })
        )
        defer { windowController.close() }

        windowController.showWindow(nil)

        let geometry = try XCTUnwrap(windowController.currentGeometry)
        let statusBarHeight = geometry.statusBarIsVisible ? geometry.statusBar.height : 0
        XCTAssertEqual(geometry.editor.height, geometry.content.height - statusBarHeight, accuracy: 0.5)
        XCTAssertGreaterThan(geometry.editor.height, 0)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        // Initial layout must use the actual native viewport, including its
        // scrollbar gutters, regardless of window placement or status bar policy.
        XCTAssertEqual(CGFloat(snapshot.info.viewport_height), surface.editorView.layoutViewportSize.height, accuracy: 0.5)
        XCTAssertEqual(snapshot.rows.count, 3)
        XCTAssertEqual(snapshot.rows.map(\.row_index), [0, 1, 2])
        XCTAssertGreaterThan(snapshot.rows[1].y, snapshot.rows[0].y)
        XCTAssertGreaterThan(snapshot.rows[2].y, snapshot.rows[1].y)
        for row in snapshot.rows {
            XCTAssertGreaterThanOrEqual(row.y, surface.viewportState.top)
            XCTAssertLessThanOrEqual(row.y + row.line_advance, surface.viewportState.top + snapshot.info.viewport_height)
        }
    }

    @MainActor
    func testLayerBackedDocumentWindowCachePaintsEveryInitialHardLine() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(
            source: Data(
                [
                    "ALPHA ROW HAS VISIBLE INK",
                    "BRAVO ROW HAS VISIBLE INK",
                    "CHARLIE ROW HAS VISIBLE INK",
                    "DELTA ROW HAS VISIBLE INK",
                ].joined(separator: "\n").utf8
            ),
            typeName: "public.plain-text"
        )
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let document = EVDocument(editorBackend: backend)
        let windowController = EVDocumentWindowController(
            document: document,
            editorSurface: surface,
            placement: EVDocumentWindowPlacement(loadFrame: { nil })
        )
        document.addWindowController(windowController)
        defer { document.close() }

        windowController.showWindow(nil)
        let window = try XCTUnwrap(windowController.window)
        window.contentView?.layoutSubtreeIfNeeded()
        surface.view.layoutSubtreeIfNeeded()
        surface.refreshPresentation()
        window.displayIfNeeded()
        surface.editorView.displayIfNeeded()

        let editorView = surface.editorView
        XCTAssertTrue(editorView.wantsLayer)
        XCTAssertNotNil(editorView.layer)
        XCTAssertNotNil(editorView.window)

        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(snapshot.rows.count, 4)
        let bands = try disjointOccupiedRowBands(snapshot: snapshot, in: editorView)
        XCTAssertEqual(bands.map(\.rowIndex), [0, 1, 2, 3])
        for pair in zip(bands, bands.dropFirst()) {
            XCTAssertLessThanOrEqual(pair.0.rect.maxY, pair.1.rect.minY)
            XCTAssertTrue(pair.0.rect.intersection(pair.1.rect).isEmpty)
        }

        let cachedDisplay = try XCTUnwrap(
            editorView.bitmapImageRepForCachingDisplay(in: editorView.bounds)
        )
        editorView.cacheDisplay(in: editorView.bounds, to: cachedDisplay)

        let background = try XCTUnwrap(
            NSColor(
                srgbRed: 1,
                green: 1,
                blue: 1,
                alpha: 1
            ).usingColorSpace(.sRGB)
        )
        let directFirstCount = nonBackgroundPixelCount(
            in: bands[0].rect,
            image: cachedDisplay,
            viewBounds: editorView.bounds,
            background: background,
            invertedY: false
        )
        let invertedFirstCount = nonBackgroundPixelCount(
            in: bands[0].rect,
            image: cachedDisplay,
            viewBounds: editorView.bounds,
            background: background,
            invertedY: true
        )
        let invertedY = invertedFirstCount > directFirstCount
        XCTAssertGreaterThan(
            max(directFirstCount, invertedFirstCount),
            0,
            "the cache must contain the first visual row so its bitmap orientation is known"
        )

        for band in bands {
            XCTAssertGreaterThan(
                nonBackgroundPixelCount(
                    in: band.rect,
                    image: cachedDisplay,
                    viewBounds: editorView.bounds,
                    background: background,
                    invertedY: invertedY
                ),
                0,
                "layer-backed cacheDisplay omitted visual row \(band.rowIndex)"
            )
        }
    }

    @MainActor
    func testLiveRefreshAfterEveryEditKeepsAllPositionedRowsCurrent() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 420)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)

        for operation in [
            { _ = try session.sendText("i") },
            { _ = try session.sendText("FIRST ROW") },
            { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER)) },
            { _ = try session.sendText("SECOND ROW") },
            { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER)) },
            { _ = try session.sendText("THIRD ROW") },
        ] {
            surface.performInput(operation)
        }

        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(snapshot.rows.count, 3)
        XCTAssertEqual(snapshot.rows.map(\.row_index), [0, 1, 2])
        XCTAssertGreaterThan(snapshot.rows[1].y, snapshot.rows[0].y)
        XCTAssertGreaterThan(snapshot.rows[2].y, snapshot.rows[1].y)
        for rowIndex in UInt64(0) ... 2 {
            XCTAssertFalse(snapshot.clusters.filter { $0.row_index == rowIndex }.isEmpty)
        }
    }

    @MainActor
    func testIncrementalEnterProducesTwoPositionedRows() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 420)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)

        _ = try session.sendText("i")
        _ = try session.sendText("Hello, Viem — SF Pro 14")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        _ = try session.sendText("Unicode: café 漢字 👩🏽‍💻")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        surface.refreshPresentation()

        XCTAssertEqual(surface.formattedText, "Hello, Viem — SF Pro 14\nUnicode: café 漢字 👩🏽‍💻")
        let layout = try session.layoutExport()
        XCTAssertGreaterThanOrEqual(layout.rows.count, 2)
        XCTAssertGreaterThan(layout.rows[1].y, layout.rows[0].y)
        XCTAssertLessThan(layout.rows[1].y, 100)

        let secondLineStart = UInt64("Hello, Viem — SF Pro 14\n".utf8.count)
        let secondLineClusters = layout.clusters.filter { $0.text_start >= secondLineStart }
        XCTAssertFalse(secondLineClusters.isEmpty)
        XCTAssertTrue(secondLineClusters.allSatisfy { $0.row_index == 1 })
        XCTAssertTrue(secondLineClusters.allSatisfy { $0.ink_bounds.y < 100 })
        XCTAssertTrue(layout.carets.contains { $0.text_offset >= secondLineStart })
    }

    @MainActor
    func testIncrementalEnterPaintsGlyphPixelsOnBothPositionedRows() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 420)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)

        _ = try session.sendText("i")
        _ = try session.sendText("FIRST ROW")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        _ = try session.sendText("SECOND ROW")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        _ = try session.sendKey(
            kind: UInt32(VIEM_KEY_CHARACTER),
            codepoint: UInt32(Character("g").asciiValue!)
        )
        _ = try session.sendKey(
            kind: UInt32(VIEM_KEY_CHARACTER),
            codepoint: UInt32(Character("g").asciiValue!)
        )
        surface.refreshPresentation()

        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(snapshot.rows.count, 2)
        let firstClusters = snapshot.clusters.filter { $0.row_index == 0 }
        let secondClusters = snapshot.clusters.filter { $0.row_index == 1 }
        XCTAssertFalse(firstClusters.isEmpty)
        XCTAssertFalse(secondClusters.isEmpty)
        XCTAssertTrue(secondClusters.allSatisfy {
            session.provider.renderRegistry.contains(
                identifier: $0.render_run.identifier,
                metricsGeneration: $0.render_run.metrics_generation
            )
        })

        let bitmap = try makeBitmap(width: 700, height: 420)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: bitmap, flipped: true)
        surface.editorView.draw(surface.editorView.bounds)
        NSGraphicsContext.restoreGraphicsState()

        let firstInk = try XCTUnwrap(inkBounds(for: firstClusters, in: surface.editorView))
        let secondInk = try XCTUnwrap(inkBounds(for: secondClusters, in: surface.editorView))
        XCTAssertGreaterThan(
            secondInk.minY,
            firstInk.maxY,
            "each regional row's exported ink bounds must be in document-global coordinates"
        )
        XCTAssertGreaterThan(nonBackgroundPixelCount(in: firstInk, bitmap: bitmap), 0)
        XCTAssertGreaterThan(
            nonBackgroundPixelCount(in: secondInk, bitmap: bitmap),
            0,
            "the second exported visual row must be painted, not merely present in core state"
        )
    }

    @MainActor
    private func inkBounds(
        for clusters: [ViemPositionedClusterV1],
        in view: EVEditorView
    ) -> NSRect? {
        clusters.reduce(nil as NSRect?) { result, cluster in
            let ink = view.viewRect(cluster.ink_bounds)
            return result.map { $0.union(ink) } ?? ink
        }?.insetBy(dx: -2, dy: -2)
    }

    @MainActor
    private func disjointOccupiedRowBands(
        snapshot: EVLayoutExport,
        in view: EVEditorView
    ) throws -> [(rowIndex: UInt64, rect: NSRect)] {
        let occupiedRows = snapshot.rows.filter { row in
            snapshot.clusters.contains { $0.row_index == row.row_index }
        }.sorted { $0.y < $1.y }
        let baselines = occupiedRows.map {
            view.viewPoint(fromLayoutPoint: NSPoint(x: 0, y: CGFloat($0.baseline))).y
        }

        return try occupiedRows.enumerated().map { index, row in
            let clusters = snapshot.clusters.filter { $0.row_index == row.row_index }
            let ink = try XCTUnwrap(inkBounds(for: clusters, in: view))
                .intersection(view.bounds)
            let top = index == 0
                ? view.viewPoint(fromLayoutPoint: NSPoint(x: 0, y: CGFloat(row.y))).y
                : (baselines[index - 1] + baselines[index]) / 2
            let bottom = index + 1 < occupiedRows.count
                ? (baselines[index] + baselines[index + 1]) / 2
                : view.viewPoint(
                    fromLayoutPoint: NSPoint(
                        x: 0,
                        y: CGFloat(row.y + max(row.line_advance, row.ascent + row.descent))
                    )
                ).y
            let band = NSRect(
                x: ink.minX,
                y: max(view.bounds.minY, top),
                width: ink.width,
                height: min(view.bounds.maxY, bottom) - max(view.bounds.minY, top)
            ).intersection(view.bounds)
            XCTAssertFalse(band.isEmpty)
            XCTAssertFalse(
                band.intersection(ink).isEmpty,
                "row \(row.row_index) band must contain its exported ink geometry"
            )
            return (row.row_index, band)
        }
    }

    private func makeBitmap(width: Int, height: Int) throws -> CGContext {
        try XCTUnwrap(
            CGContext(
                data: nil,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
        )
    }

    private func nonBackgroundPixelCount(in rect: NSRect, bitmap: CGContext) -> Int {
        guard let storage = bitmap.data?.assumingMemoryBound(to: UInt8.self) else { return 0 }
        let width = bitmap.width
        let height = bitmap.height
        let bytesPerRow = bitmap.bytesPerRow
        let reference = (storage[0], storage[1], storage[2], storage[3])
        let minX = max(0, Int(floor(rect.minX)))
        let maxX = min(width, Int(ceil(rect.maxX)))
        let minY = max(0, Int(floor(rect.minY)))
        let maxY = min(height, Int(ceil(rect.maxY)))

        func count(inverted: Bool) -> Int {
            var changed = 0
            for viewY in minY ..< maxY {
                let pixelY = inverted ? height - viewY - 1 : viewY
                for x in minX ..< maxX {
                    let offset = pixelY * bytesPerRow + x * 4
                    if storage[offset] != reference.0
                        || storage[offset + 1] != reference.1
                        || storage[offset + 2] != reference.2
                        || storage[offset + 3] != reference.3
                    {
                        changed += 1
                    }
                }
            }
            return changed
        }

        // CGContext bitmap storage is bottom-up, while EVEditorView is
        // flipped. Taking the larger count also keeps this probe stable if
        // NSGraphicsContext changes its backing orientation.
        return max(count(inverted: false), count(inverted: true))
    }

    private func nonBackgroundPixelCount(
        in rect: NSRect,
        image: NSBitmapImageRep,
        viewBounds: NSRect,
        background: NSColor,
        invertedY: Bool
    ) -> Int {
        guard viewBounds.width > 0, viewBounds.height > 0 else { return 0 }
        let scaleX = CGFloat(image.pixelsWide) / viewBounds.width
        let scaleY = CGFloat(image.pixelsHigh) / viewBounds.height
        let minX = max(0, Int(floor((rect.minX - viewBounds.minX) * scaleX)))
        let maxX = min(
            image.pixelsWide,
            Int(ceil((rect.maxX - viewBounds.minX) * scaleX))
        )
        let rawMinY = max(0, Int(floor((rect.minY - viewBounds.minY) * scaleY)))
        let rawMaxY = min(
            image.pixelsHigh,
            Int(ceil((rect.maxY - viewBounds.minY) * scaleY))
        )

        var changed = 0
        for rawY in rawMinY ..< rawMaxY {
            let pixelY = invertedY ? image.pixelsHigh - rawY - 1 : rawY
            guard pixelY >= 0, pixelY < image.pixelsHigh else { continue }
            for x in minX ..< maxX {
                guard let color = image.colorAt(x: x, y: pixelY)?.usingColorSpace(.sRGB) else {
                    continue
                }
                let delta = abs(color.redComponent - background.redComponent)
                    + abs(color.greenComponent - background.greenComponent)
                    + abs(color.blueComponent - background.blueComponent)
                    + abs(color.alphaComponent - background.alphaComponent)
                if delta > 0.2 { changed += 1 }
            }
        }
        return changed
    }
}
