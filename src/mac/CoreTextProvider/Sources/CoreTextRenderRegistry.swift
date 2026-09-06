import AppKit
import CoreGraphics
import CoreText
import Foundation

/// Provider-owned Core Text draw data referenced by the integer handles that
/// cross the Rust C ABI. The registry, rather than core, owns every native font
/// and glyph buffer for the complete metrics-generation lifetime.
public final class CoreTextRenderRegistry: @unchecked Sendable {
  struct GlyphBatch {
    let font: CTFont
    let glyphs: [CGGlyph]
    let positions: [CGPoint]
  }

  struct Resource {
    let signature: [UInt8]
    let batches: [GlyphBatch]
    let isColorGlyph: Bool
  }

  private let lock = NSLock()
  private var generation: UInt64
  private var resources: [UInt64: Resource] = [:]

  init(generation: UInt64) {
    self.generation = generation
  }

  /// Retires every native object from older layout snapshots. Call this only
  /// when the provider's advertised metrics generation changes, or when its
  /// owning view is detached from core.
  public func retireAll(forNewGeneration newGeneration: UInt64) {
    lock.lock()
    generation = newGeneration
    resources.removeAll(keepingCapacity: true)
    lock.unlock()
  }

  public func contains(identifier: UInt64, metricsGeneration: UInt64) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return generation == metricsGeneration && resources[identifier] != nil
  }

  /// True when at least one font participating in this shaped cluster has a
  /// native bitmap/COLR/SVG color-glyph table. The editor uses this to choose
  /// the non-destructive translucent Normal-mode block treatment.
  public func isColorGlyph(identifier: UInt64, metricsGeneration: UInt64) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return generation == metricsGeneration && resources[identifier]?.isColorGlyph == true
  }

  /// Returns the normalized OpenType settings from the resolved font retained
  /// for a render run. This is intentionally internal test visibility: core
  /// owns the portable settings, while Core Text remains the rendering owner.
  func openTypeFeatureSettings(
    identifier: UInt64,
    metricsGeneration: UInt64
  ) -> [(tag: String, value: UInt32)]? {
    lock.lock()
    let font = generation == metricsGeneration
      ? resources[identifier]?.batches.first?.font
      : nil
    lock.unlock()
    guard let font else { return nil }

    guard
      let settings = CTFontCopyAttribute(font, kCTFontFeatureSettingsAttribute)
        as? [[CFString: Any]]
    else { return [] }
    return settings.compactMap { setting in
      guard let tag = setting[kCTFontOpenTypeFeatureTag] as? String,
        let value = setting[kCTFontOpenTypeFeatureValue] as? NSNumber
      else { return nil }
      return (tag: tag, value: value.uint32Value)
    }
  }

  /// Returns the advance of a space in the font that shaped this render run.
  /// The editor uses this only for the explicit empty/end-of-line caret
  /// geometry where there is no glyph beneath the cursor to supply a width.
  public func spaceAdvance(identifier: UInt64, metricsGeneration: UInt64) -> CGFloat? {
    lock.lock()
    let font = generation == metricsGeneration
      ? resources[identifier]?.batches.first?.font
      : nil
    lock.unlock()
    guard let font else { return nil }

    var character = UniChar(0x20)
    var glyph = CGGlyph()
    guard CTFontGetGlyphsForCharacters(font, &character, &glyph, 1), glyph != 0 else {
      return nil
    }
    var advance = CGSize.zero
    CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)
    let width = abs(advance.width)
    return width.isFinite && width > 0 ? width : nil
  }

  /// Draws one shaped cluster at a baseline expressed in the editor view's
  /// flipped (y-down) coordinate system. `color` supplies the document's
  /// paint-only foreground property, which deliberately is not part of the
  /// shaping ABI. Color-glyph fonts retain their native color behavior.
  @discardableResult
  public func draw(
    identifier: UInt64,
    metricsGeneration: UInt64,
    atBaseline baseline: CGPoint,
    color: NSColor,
    in context: CGContext,
    clip: CGRect? = nil
  ) -> Bool {
    lock.lock()
    let resource = generation == metricsGeneration ? resources[identifier] : nil
    lock.unlock()
    guard let resource else { return false }

    context.saveGState()
    if let clip {
      context.clip(to: clip)
    }
    context.setFillColor(color.cgColor)
    context.setTextDrawingMode(.fill)
    context.textMatrix = .identity
    context.translateBy(x: baseline.x, y: baseline.y)
    context.scaleBy(x: 1, y: -1)

    for batch in resource.batches {
      batch.glyphs.withUnsafeBufferPointer { glyphBuffer in
        batch.positions.withUnsafeBufferPointer { positionBuffer in
          guard let glyphBase = glyphBuffer.baseAddress,
            let positionBase = positionBuffer.baseAddress
          else { return }
          CTFontDrawGlyphs(
            batch.font,
            glyphBase,
            positionBase,
            batch.glyphs.count,
            context
          )
        }
      }
    }
    context.restoreGState()
    return true
  }

  func install(_ resource: Resource, preferredIdentifier: UInt64, generation: UInt64) -> UInt64? {
    lock.lock()
    defer { lock.unlock() }
    guard self.generation == generation else { return nil }

    var identifier = preferredIdentifier == 0 ? 1 : preferredIdentifier
    while let existing = resources[identifier] {
      if existing.signature == resource.signature {
        return identifier
      }
      identifier = identifier &+ 1
      if identifier == 0 { identifier = 1 }
    }
    resources[identifier] = resource
    return identifier
  }
}
