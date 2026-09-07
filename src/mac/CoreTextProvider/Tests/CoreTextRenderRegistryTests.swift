import AppKit
import CoreGraphics
import CoreText
import Testing

@testable import EvimCoreTextProvider

@Suite("Core Text render registry")
struct CoreTextRenderRegistryTests {
  @Test("Cached paint colors preserve colored ink, synthetic stroke and transparency")
  func documentColorAndSyntheticStroke() throws {
    let registry = CoreTextRenderRegistry(generation: 1)
    let font = CTFontCreateWithName("TimesNewRomanPSMT" as CFString, 32, nil)
    var character = UniChar(0x48)
    var glyph = CGGlyph()
    #expect(CTFontGetGlyphsForCharacters(font, &character, &glyph, 1))
    for (identifier, stroke) in [(UInt64(1), CGFloat(0)), (UInt64(2), CGFloat(1.5))] {
      #expect(registry.install(.init(
        signature: [UInt8(identifier)],
        batches: [.init(font: font, strokeWidth: stroke, glyphs: [glyph], positions: [.zero])],
        isColorGlyph: false
      ), preferredIdentifier: identifier, generation: 1) == identifier)
    }

    let color = NSColor(srgbRed: 0.8, green: 0.2, blue: 0.05, alpha: 0.65)
    var inkCounts: [Int] = []
    for identifier in [UInt64(1), UInt64(2)] {
      let native = try bitmap()
      let cached = try bitmap()
      #expect(registry.draw(identifier: identifier, metricsGeneration: 1,
        atBaseline: CGPoint(x: 8, y: 44), color: color, in: native))
      #expect(registry.draw(identifier: identifier, metricsGeneration: 1,
        atBaseline: CGPoint(x: 8, y: 44), color: color.cgColor, in: cached))
      let pixels = try bytes(cached)
      let nativePixels = try bytes(native)
      #expect(pixels == nativePixels)
      let ink = stride(from: 0, to: pixels.count, by: 4).filter { pixels[$0 + 3] > 16 }
      #expect(!ink.isEmpty)
      // Both the face and its synthetic outline must retain the document's
      // red foreground, including alpha, rather than a context/default color.
      #expect(ink.allSatisfy { pixels[$0] > pixels[$0 + 1] && pixels[$0 + 1] >= pixels[$0 + 2] })
      #expect(ink.contains { pixels[$0 + 3] < 255 })
      inkCounts.append(ink.count)
    }
    #expect(inkCounts[1] > inkCounts[0], "Synthetic bold must still expand the glyph's ink")

    let transparent = try bitmap()
    #expect(registry.draw(identifier: 2, metricsGeneration: 1,
      atBaseline: CGPoint(x: 8, y: 44), color: NSColor.clear.cgColor, in: transparent))
    #expect(try bytes(transparent).allSatisfy { $0 == 0 })
  }

  @Test("Native color glyphs retain their palette with a cached foreground")
  func colorGlyphPalette() throws {
    let font = CTFontCreateWithName("AppleColorEmoji" as CFString, 32, nil)
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: "😀", attributes: [
      NSAttributedString.Key(kCTFontAttributeName as String): font,
    ]))
    let runs = CTLineGetGlyphRuns(line) as! [CTRun]
    let batches = runs.map { run -> CoreTextRenderRegistry.GlyphBatch in
      let count = CTRunGetGlyphCount(run)
      var glyphs = Array(repeating: CGGlyph(), count: count)
      var positions = Array(repeating: CGPoint.zero, count: count)
      CTRunGetGlyphs(run, CFRange(location: 0, length: 0), &glyphs)
      CTRunGetPositions(run, CFRange(location: 0, length: 0), &positions)
      return .init(font: font, strokeWidth: 0, glyphs: glyphs, positions: positions)
    }
    let registry = CoreTextRenderRegistry(generation: 1)
    #expect(registry.install(.init(signature: [1], batches: batches, isColorGlyph: true),
      preferredIdentifier: 1, generation: 1) == 1)
    let context = try bitmap()
    #expect(registry.draw(identifier: 1, metricsGeneration: 1,
      atBaseline: CGPoint(x: 8, y: 44), color: NSColor.blue.cgColor, in: context))
    let pixels = try bytes(context)
    #expect(stride(from: 0, to: pixels.count, by: 4).contains {
      pixels[$0 + 3] > 128 && pixels[$0] > 100 && pixels[$0 + 1] > 70 && pixels[$0 + 2] < 80
    }, "The emoji must retain yellow native ink rather than being tinted blue")
  }

  private func bitmap() throws -> CGContext {
    try #require(CGContext(data: nil, width: 96, height: 64, bitsPerComponent: 8,
      bytesPerRow: 96 * 4, space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
  }

  private func bytes(_ context: CGContext) throws -> [UInt8] {
    let data = try #require(context.data).assumingMemoryBound(to: UInt8.self)
    return Array(UnsafeBufferPointer(start: data, count: 96 * 64 * 4))
  }
}
