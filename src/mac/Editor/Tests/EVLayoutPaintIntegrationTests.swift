import AppKit
import CEvimCore
import EvimAppShell
import EvimCoreTextProvider
import XCTest

@testable import EvimEditor

final class EVLayoutPaintIntegrationTests: XCTestCase {
  @MainActor
  func testDefaultPaintUsesApplicationThemeAndSFProFourteen() throws {
    let prior = EVThemeStore.shared.theme
    EVThemeStore.shared.update(.paper)
    defer { EVThemeStore.shared.update(prior) }
    let surface = try makeSurface(text: "Paint")
    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
    let paint = try XCTUnwrap(surface.layoutPaint)

    XCTAssertTrue(paint.info.identity.isSameLayout(as: snapshot.info.identity))
    XCTAssertEqual(CoreTextMeasurementProvider.defaultFontFamily, "SF Pro")
    XCTAssertEqual(CoreTextMeasurementProvider.defaultFontSize, 14)
    assertRGBA(paint.info.canvas_background, red: 1, green: 1, blue: 1, alpha: 1)
    assertRGBA(paint.info.default_paint.foreground, red: 0, green: 0, blue: 0, alpha: 1)
    XCTAssertEqual(paint.info.default_paint.flags, UInt32(EVIM_TEXT_PAINT_DEFAULT_FOREGROUND))
    XCTAssertEqual(paint.info.flags, UInt32(EVIM_LAYOUT_PAINT_DEFAULT_CANVAS))
    XCTAssertTrue(paint.runs.isEmpty)

    let image = try render(surface.editorView)
    let corner = try XCTUnwrap(image.colorAt(x: 1, y: 1)?.usingColorSpace(.sRGB))
    XCTAssertEqual(corner.redComponent, 1, accuracy: 0.02)
    XCTAssertEqual(corner.greenComponent, 1, accuracy: 0.02)
    XCTAssertEqual(corner.blueComponent, 1, accuracy: 0.02)
    let ink = try XCTUnwrap(unionOfInkBounds(snapshot.clusters, in: surface.editorView))
    XCTAssertTrue(
      colors(in: ink, image: image).contains { color in
        color.redComponent < 0.35 && color.greenComponent < 0.35 && color.blueComponent < 0.35
      })
  }

  @MainActor
  func testMixedPaintRunRendersCanvasBackgroundForegroundAndDecorations() throws {
    let surface = try makeSurface(text: "A ")
    let snapshot = try XCTUnwrap(surface.layoutSnapshot)
    XCTAssertEqual(snapshot.clusters.count, 2)
    let second = snapshot.clusters[1]

    var info = try XCTUnwrap(surface.layoutPaint).info
    info.flags = 0
    info.canvas_background = rgba(red: 0.1, green: 0.2, blue: 0.3, alpha: 1)
    info.default_paint = textPaint(foreground: rgba(red: 0, green: 0, blue: 0, alpha: 1))
    info.paint_run_count = 1
    var run = EvimPaintStyleRunV1()
    run.struct_size = UInt32(MemoryLayout<EvimPaintStyleRunV1>.size)
    run.text_start = second.text_start
    run.text_end = second.text_end
    run.paint = textPaint(
      flags: UInt32(EVIM_TEXT_PAINT_HAS_BACKGROUND)
        | UInt32(EVIM_TEXT_PAINT_UNDERLINE)
        | UInt32(EVIM_TEXT_PAINT_STRIKETHROUGH),
      foreground: rgba(red: 1, green: 0, blue: 0, alpha: 1),
      background: rgba(red: 1, green: 1, blue: 0, alpha: 1)
    )
    let mixed = EVLayoutPaintExport(info: info, runs: [run])
    surface.layoutPaint = mixed

    let firstResolved = surface.editorView.resolvedTextPaint(
      for: snapshot.clusters[0],
      paint: mixed
    )
    let secondResolved = surface.editorView.resolvedTextPaint(for: second, paint: mixed)
    XCTAssertNil(firstResolved.background)
    XCTAssertNotNil(secondResolved.background)
    XCTAssertTrue(secondResolved.underline)
    XCTAssertTrue(secondResolved.strikethrough)
    let decorations = surface.editorView.textDecorationsForDrawing(
      in: snapshot,
      paint: mixed
    )
    XCTAssertEqual(decorations.map(\.kind), [.underline, .strikethrough])
    let expectedWidth = CGFloat(second.typographic_bounds.width)
    XCTAssertTrue(decorations.allSatisfy { abs($0.rect.width - expectedWidth) < 0.01 })

    let image = try render(surface.editorView)
    let corner = try XCTUnwrap(image.colorAt(x: 1, y: 1)?.usingColorSpace(.sRGB))
    XCTAssertEqual(corner.redComponent, 0.1, accuracy: 0.03)
    XCTAssertEqual(corner.greenComponent, 0.2, accuracy: 0.03)
    XCTAssertEqual(corner.blueComponent, 0.3, accuracy: 0.03)

    let secondBounds = surface.editorView.viewRect(second.typographic_bounds).insetBy(
      dx: -1, dy: -1)
    let secondColors = colors(in: secondBounds, image: image)
    XCTAssertTrue(
      secondColors.contains { color in
        color.redComponent > 0.8 && color.greenComponent > 0.8 && color.blueComponent < 0.2
      }, "the override background must paint behind the second cluster")
    XCTAssertTrue(
      secondColors.contains { color in
        color.redComponent > 0.65
          && color.greenComponent < 0.35
          && color.blueComponent < 0.35
      }, "the override foreground/decorations must render in red")
    for decoration in decorations {
      XCTAssertTrue(
        colors(in: decoration.rect.insetBy(dx: -1, dy: -1), image: image).contains {
          $0.redComponent > 0.65 && $0.greenComponent < 0.35 && $0.blueComponent < 0.35
        }, "\(decoration.kind) must paint across the styled space cluster")
    }
  }

  @MainActor
  func testChangingThemeColorsRepaintsWithoutChangingSourceOrLayout() throws {
    let prior = EVThemeStore.shared.theme
    defer { EVThemeStore.shared.update(prior) }
    let surface = try makeSurface(text: "Theme")
    let before = try XCTUnwrap(surface.layoutSnapshot)
    let source = try surface.backend.serializedSource(typeName: "public.plain-text")
    let state = surface.backend.persistenceState
    var midnight = EVTheme.midnight
    midnight.padding = prior.padding
    EVThemeStore.shared.update(midnight)
    let after = try XCTUnwrap(surface.layoutSnapshot)
    XCTAssertTrue(before.info.identity.isSameLayout(as: after.info.identity))
    XCTAssertEqual(surface.backend.persistenceState, state)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: "public.plain-text"), source)
    let paint = try XCTUnwrap(surface.layoutPaint)
    let first = try XCTUnwrap(after.clusters.first)
    let color = try XCTUnwrap(
      surface.editorView.resolvedTextPaint(for: first, paint: paint).foreground.usingColorSpace(
        .sRGB))
    XCTAssertEqual(color.redComponent, midnight.foreground.red, accuracy: 0.001)
    let image = try render(surface.editorView)
    let corner = try XCTUnwrap(image.colorAt(x: 1, y: 1)?.usingColorSpace(.sRGB))
    XCTAssertEqual(corner.blueComponent, midnight.background.blue, accuracy: 0.02)
  }

  @MainActor
  func testEmptyDocumentCaretWidthUsesCurrentTypingFont() throws {
    let surface = try makeSurface(text: "")
    let session = try XCTUnwrap(surface.session)
    XCTAssertEqual(try session.currentFontEnWidth(), 7, accuracy: 0.001)
    let editor = EVStyleEditorViewController()
    editor.retarget(document: surface, styleKey: .baseDocument)
    XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(48)))
    XCTAssertEqual(try session.currentFontEnWidth(), 24, accuracy: 0.001)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: "public.plain-text"), Data())
  }

  @MainActor
  private func makeSurface(text: String) throws -> EVEditorSurfaceController {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 160)
    surface.viewDidLayout()
    return surface
  }

  @MainActor
  private func render(_ view: EVEditorView) throws -> NSBitmapImageRep {
    let width = Int(view.bounds.width)
    let height = Int(view.bounds.height)
    let bitmap = try XCTUnwrap(
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
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: bitmap, flipped: true)
    view.draw(view.bounds)
    NSGraphicsContext.restoreGraphicsState()
    return NSBitmapImageRep(cgImage: try XCTUnwrap(bitmap.makeImage()))
  }

  @MainActor
  private func unionOfInkBounds(
    _ clusters: [EvimPositionedClusterV1],
    in view: EVEditorView
  ) -> NSRect? {
    clusters.reduce(nil as NSRect?) { result, cluster in
      let rect = view.viewRect(cluster.ink_bounds)
      return result.map { $0.union(rect) } ?? rect
    }?.insetBy(dx: -1, dy: -1)
  }

  private func colors(in rect: NSRect, image: NSBitmapImageRep) -> [NSColor] {
    let minX = max(0, Int(floor(rect.minX)))
    let maxX = min(image.pixelsWide, Int(ceil(rect.maxX)))
    let minY = max(0, Int(floor(rect.minY)))
    let maxY = min(image.pixelsHigh, Int(ceil(rect.maxY)))
    var result: [NSColor] = []
    for viewY in minY..<maxY {
      for pixelY in [viewY, image.pixelsHigh - viewY - 1] where pixelY >= 0 {
        for x in minX..<maxX {
          if let color = image.colorAt(x: x, y: pixelY)?.usingColorSpace(.sRGB) {
            result.append(color)
          }
        }
      }
    }
    return result
  }

  private func rgba(red: Float, green: Float, blue: Float, alpha: Float) -> EvimRgbaV1 {
    var value = EvimRgbaV1()
    value.red = red
    value.green = green
    value.blue = blue
    value.alpha = alpha
    return value
  }

  private func textPaint(
    flags: UInt32 = 0,
    foreground: EvimRgbaV1,
    background: EvimRgbaV1 = EvimRgbaV1()
  ) -> EvimTextPaintV1 {
    var value = EvimTextPaintV1()
    value.struct_size = UInt32(MemoryLayout<EvimTextPaintV1>.size)
    value.flags = flags
    value.foreground = foreground
    value.background = background
    return value
  }

  private func assertRGBA(
    _ actual: EvimRgbaV1,
    red: Float,
    green: Float,
    blue: Float,
    alpha: Float,
    file: StaticString = #filePath,
    line: UInt = #line
  ) {
    XCTAssertEqual(actual.red, red, accuracy: 0.001, file: file, line: line)
    XCTAssertEqual(actual.green, green, accuracy: 0.001, file: file, line: line)
    XCTAssertEqual(actual.blue, blue, accuracy: 0.001, file: file, line: line)
    XCTAssertEqual(actual.alpha, alpha, accuracy: 0.001, file: file, line: line)
  }
}
