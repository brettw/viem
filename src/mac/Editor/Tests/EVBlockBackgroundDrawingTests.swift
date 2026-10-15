import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBlockBackgroundDrawingTests: XCTestCase {

    func testQuotedTableHasOneContinuousOuterBorderAndBackground() throws {
        let prior = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(prior) }
        let source = "> | Quoted item | Value |\n> | --- | ---: |\n> | Inside the quotation | 7 |"
        let surface = try makeSurface(source)
        surface.view.frame = NSRect(x: 0, y: 0, width: 1000, height: 600)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        let quote = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))
        for (property, value) in [
            (EVStyleProperty.blockBorderLeftWidth, EVStyleValue.float(4)),
            (.blockBorderLeftColor, .color(.init(red: 1, green: 0, blue: 0, alpha: 1))),
            (.blockPaddingLeft, .float(12)), (.blockPaddingTop, .float(8)),
            (.blockPaddingBottom, .float(8)),
            (.blockBackground, .color(.init(red: 1, green: 0, blue: 0, alpha: 0.5))),
        ] {
            try session.editStyle(key: quote, expected: surface.backend.styleSheetSnapshot().identity,
                mutation: .setDeclaration(property, value))
        }
        for name in ["Table", "Table cell", "Table header"] {
            let key = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: name))
            for (property, value) in [
                (EVStyleProperty.blockBackground, EVStyleValue.color(.init(red: 0, green: 0, blue: 1, alpha: 1))),
                (.blockPaddingTop, .float(6)), (.blockPaddingBottom, .float(6)),
                (.blockBorderTopWidth, .float(1)), (.blockBorderBottomWidth, .float(1)),
            ] {
                try session.editStyle(key: key, expected: surface.backend.styleSheetSnapshot().identity,
                    mutation: .setDeclaration(property, value))
            }
        }
        for scale: CGFloat in [1, 1.25] {
            _ = try session.setScale(scale)
            surface.refreshPresentation()
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let borders = snapshot.decorations.filter {
                $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0
            }.map { surface.editorView.viewRect($0.typographic_bounds) }.sorted { $0.minY < $1.minY }
            XCTAssertEqual(borders.count, 2, "Only one outer quote slice per table row; no cell-local bars")
            let first = try XCTUnwrap(borders.first)
            let last = try XCTUnwrap(borders.last)
            XCTAssertEqual(first.minX, last.minX, accuracy: 0.001)
            XCTAssertEqual(first.maxY, last.minY, accuracy: 0.001)
            let cells = snapshot.tableCells.map { surface.editorView.viewRect($0.rect) }
            let grid = cells.reduce(CGRect.null) { $0.union($1) }
            XCTAssertLessThan(first.maxX, grid.minX)
            XCTAssertLessThan(first.minY, grid.minY)
            XCTAssertGreaterThan(last.maxY, grid.maxY)
            for backingScale: CGFloat in [1, 2] {
                let image = try bitmap(surface.editorView, backingScale: backingScale)
                let red = try composite([NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)])
                let pink = try composite([NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 0.5)])
                for y in Int(ceil(first.minY * backingScale))..<Int(floor(last.maxY * backingScale)) {
                    assertPixel(image, at: CGPoint(x: first.midX * backingScale, y: CGFloat(y)), equals: red)
                    assertPixel(image, at: CGPoint(x: (first.maxX + 4 * scale) * backingScale, y: CGFloat(y)), equals: pink)
                }
                let blue = try composite([NSColor(srgbRed: 0, green: 0, blue: 1, alpha: 1)])
                for cell in cells {
                    // The quote background is behind the opaque cell fill.
                    assertPixel(image, at: CGPoint(x: cell.midX * backingScale,
                        y: (cell.minY + 3 * scale) * backingScale), equals: blue)
                }
            }
        }
        XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testNestedQuoteBackgroundsCompositeOnceOverTheirParents() throws {
        let prior = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(prior) }
        let source = "> Outer paragraph\n>\n>> Inner paragraph"
        for alpha: Float in [1, 0.5, 0] {
            let surface = try makeSurface(source)
            let session = try XCTUnwrap(surface.session)
            let key = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))
            for (property, value) in [
                (EVStyleProperty.blockBackground, EVStyleValue.color(.init(red: 1, green: 0, blue: 0, alpha: alpha))),
                (.blockPaddingLeft, .float(16)), (.blockPaddingTop, .float(16)),
                (.blockPaddingBottom, .float(16)), (.blockPaddingRight, .float(16)),
            ] {
                try session.editStyle(key: key, expected: surface.backend.styleSheetSnapshot().identity,
                    mutation: .setDeclaration(property, value))
            }
            surface.refreshPresentation()
            let fills = try XCTUnwrap(surface.layoutSnapshot).decorations.filter {
                $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0
            }.map { surface.editorView.viewRect($0.typographic_bounds) }.sorted { $0.minX < $1.minX }
            let outer = try XCTUnwrap(fills.first)
            let inner = try XCTUnwrap(fills.last)
            XCTAssertGreaterThan(inner.minX, outer.minX)
            let image = try bitmap(surface.editorView)
            let red = NSColor(srgbRed: 1, green: 0, blue: 0, alpha: CGFloat(alpha))
            assertPixel(image, at: CGPoint(x: outer.minX + 6, y: outer.minY + 6), equals: try composite([red]))
            assertPixel(image, at: CGPoint(x: inner.minX + 6, y: inner.minY + 6), equals: try composite([red, red]))
            XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        }
    }

    func testCodeContainerBackgroundUpdatesInDocumentAndStylePreview() throws {
        let prior = EVThemeStore.shared.theme
        EVThemeStore.shared.update(.paper)
        defer { EVThemeStore.shared.update(prior) }
        let source = "Before\n\n```\nfirst line\n\nlast line\n```\n\nAfter"
        for type in [EVDocument.markdownType] {
            let surface = try makeSurface(source, typeName: type)
            let editor = EVStyleEditorViewController()
            let key = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Code Block"))
            editor.retarget(document: surface, styleKey: key)
            XCTAssertEqual(editor.inspection.selectedKind, .codeBlock)
            // Change only paint first, after the initial layout is already cached.
            for alpha: Float in [1, 0.5, 0] {
                let background = EVStyleColor(red: 1, green: 0, blue: 0, alpha: alpha)
                XCTAssertTrue(editor.setPropertyForTesting(.blockBackground, value: .color(background)))
                let snapshot = try XCTUnwrap(surface.layoutSnapshot)
                let fills = snapshot.decorations.filter { $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0 }
                XCTAssertFalse(fills.isEmpty, type)
                let fill = try XCTUnwrap(fills.first)
                XCTAssertEqual(fill.paint.foreground.alpha, alpha)
                let rect = surface.editorView.viewRect(fill.typographic_bounds)
                assertPixel(try bitmap(surface.editorView), at: CGPoint(x: rect.maxX - 4, y: rect.midY),
                    equals: try composite([NSColor(srgbRed: 1, green: 0, blue: 0, alpha: CGFloat(alpha))]))
                let preview = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
                XCTAssertNil(preview.blockPreviewError)
                XCTAssertFalse(preview.boxes.filter(\.isBackground).isEmpty)
                XCTAssertTrue(preview.boxes.filter(\.isBackground).allSatisfy { $0.color == background })
                let previewView = try XCTUnwrap(findPreview(in: editor.view))
                previewView.frame = NSRect(x: 0, y: 0, width: 560, height: 400)
                let previewRect = try XCTUnwrap(preview.boxes.first { $0.isBackground }).rect
                assertPixel(try bitmap(previewView), at: CGPoint(x: previewRect.maxX - 4, y: previewRect.midY),
                    equals: try composite([NSColor(srgbRed: 1, green: 0, blue: 0, alpha: CGFloat(alpha))]))
            }
            XCTAssertTrue(editor.setPropertyForTesting(.blockBackground,
                value: .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 0.5))))
            _ = try XCTUnwrap(surface.session).setScale(1.25)
            surface.refreshPresentation()
            let fills = try XCTUnwrap(surface.layoutSnapshot).decorations.filter {
                $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0
            }.map { surface.editorView.viewRect($0.typographic_bounds) }
            let extent = fills.reduce(CGRect.null) { $0.union($1) }
            let expected = try composite([NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 0.5)])
            // Adjacent slices of one code owner must not leave seams or blend
            // twice, including blank lines and fractional zoom coordinates.
            for backingScale: CGFloat in [1, 2] {
                let zoomedImage = try bitmap(surface.editorView, backingScale: backingScale)
                for y in Int(ceil(extent.minY * backingScale))..<Int(floor(extent.maxY * backingScale)) {
                    assertPixel(zoomedImage, at: CGPoint(x: (extent.maxX - 4) * backingScale, y: CGFloat(y)), equals: expected)
                }
            }
            XCTAssertEqual(try surface.backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    func testPartialRepaintOfTranslucentBackgroundMatchesFullRepaint() throws {
        let surface = try makeSurface("> Outer\n>\n>> Text")
        let session = try XCTUnwrap(surface.session)
        let key = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))
        try session.editStyle(key: key, expected: surface.backend.styleSheetSnapshot().identity,
            mutation: .setDeclaration(.blockBackground, .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 0.5))))
        surface.refreshPresentation()
        let context = try context(surface.editorView)
        draw(surface.editorView, into: context, dirty: surface.editorView.bounds)
        let before = try XCTUnwrap(context.makeImage().flatMap { $0.dataProvider?.data }) as Data
        draw(surface.editorView, into: context, dirty: NSRect(x: 12, y: 12, width: 170, height: 90))
        let after = try XCTUnwrap(context.makeImage().flatMap { $0.dataProvider?.data }) as Data
        XCTAssertEqual(before, after)
    }

    private func makeSurface(_ source: String, typeName requestedType: String? = nil) throws -> EVEditorSurfaceController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-background-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data(source.utf8), typeName: requestedType ?? EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 400)
        surface.viewDidLayout()
        return surface
    }

    private func findPreview(in view: NSView) -> EVCoreTextStylePreviewView? {
        if let preview = view as? EVCoreTextStylePreviewView { return preview }
        return view.subviews.lazy.compactMap { self.findPreview(in: $0) }.first
    }

    private func context(_ view: NSView, backingScale: CGFloat = 1) throws -> CGContext {
        let context = try XCTUnwrap(CGContext(data: nil, width: Int(view.bounds.width * backingScale), height: Int(view.bounds.height * backingScale),
            bitsPerComponent: 8, bytesPerRow: Int(view.bounds.width * backingScale) * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.scaleBy(x: backingScale, y: backingScale)
        return context
    }

    private func draw(_ view: NSView, into context: CGContext, dirty: NSRect) {
        context.saveGState()
        context.translateBy(x: 0, y: view.bounds.height)
        context.scaleBy(x: 1, y: -1)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        // Container compositing must not depend on the caller's drawing mode.
        NSGraphicsContext.current?.compositingOperation = .copy
        view.draw(dirty)
        NSGraphicsContext.restoreGraphicsState()
        context.restoreGState()
    }

    private func bitmap(_ view: NSView, backingScale: CGFloat = 1) throws -> NSBitmapImageRep {
        let context = try context(view, backingScale: backingScale)
        draw(view, into: context, dirty: view.bounds)
        return NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
    }

    // Render the reference through the same color-managed bitmap path. Comparing
    // raw sRGB component arithmetic would incorrectly assume a device color space.
    private func composite(_ layers: [NSColor]) throws -> NSColor {
        let view = NSView(frame: NSRect(x: 0, y: 0, width: 2, height: 2))
        let context = try context(view)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSColor(srgbRed: 1, green: 1, blue: 1, alpha: 1).setFill()
        view.bounds.fill(using: .copy)
        for color in layers {
            color.setFill()
            view.bounds.fill(using: .sourceOver)
        }
        let image = NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
        return try XCTUnwrap(image.colorAt(x: 0, y: 0)?.usingColorSpace(.sRGB))
    }

    private func assertPixel(_ image: NSBitmapImageRep, at point: CGPoint, equals expected: NSColor,
                             file: StaticString = #filePath, line: UInt = #line) {
        guard let color = image.colorAt(x: Int(point.x), y: Int(point.y))?.usingColorSpace(.sRGB) else {
            XCTFail("Missing pixel at \(point)", file: file, line: line); return
        }
        XCTAssertEqual(color.redComponent, expected.redComponent, accuracy: 0.01, "at \(point)", file: file, line: line)
        XCTAssertEqual(color.greenComponent, expected.greenComponent, accuracy: 0.01, "at \(point)", file: file, line: line)
        XCTAssertEqual(color.blueComponent, expected.blueComponent, accuracy: 0.01, "at \(point)", file: file, line: line)
        XCTAssertEqual(color.alphaComponent, 1, accuracy: 0.01, "at \(point)", file: file, line: line)
    }
}
