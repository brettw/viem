import AppKit
import CoreGraphics
import CoreText
import Foundation

/// Immutable shaping attributes that are separate from the resolved font.
/// Values are in the same scaled layout units as the retained glyph data.
public struct CoreTextRenderAttributes: Equatable {
  public let letterSpacing: CGFloat
  public let language: String?
  public let writingDirection: NSWritingDirection

  public init(letterSpacing: CGFloat = 0,
    language: String? = nil, writingDirection: NSWritingDirection = .natural) {
    self.letterSpacing = letterSpacing
    self.language = language
    self.writingDirection = writingDirection
  }
}

/// Provider-owned Core Text draw data referenced by the integer handles that
/// cross the Rust C ABI. The registry, rather than core, owns every native font
/// and glyph buffer while a response arena, layout cache, or snapshot leases it.
public final class CoreTextRenderRegistry: @unchecked Sendable {
  struct GlyphBatch {
    let font: CTFont
    let strokeWidth: CGFloat
    let glyphs: [CGGlyph]
    let positions: [CGPoint]
  }

  struct Resource {
    let signature: [UInt8]
    let batches: [GlyphBatch]
    let isColorGlyph: Bool
    let textAttributes: CoreTextRenderAttributes

    init(signature: [UInt8], batches: [GlyphBatch], isColorGlyph: Bool,
      textAttributes: CoreTextRenderAttributes = .init()) {
      self.signature = signature
      self.batches = batches
      self.isColorGlyph = isColorGlyph
      self.textAttributes = textAttributes
    }
  }

  private let lock = NSLock()
  private var generation: UInt64
  private var resources: [UInt64: Resource] = [:]
  private var references: [UInt64: Int] = [:]

  var resourceCountForTesting: Int {
    lock.lock(); defer { lock.unlock() }; return resources.count
  }

  var storageCapacityForTesting: Int {
    lock.lock(); defer { lock.unlock() }; return max(resources.capacity, references.capacity)
  }

  /// Registry estimates include its owned arrays; Core Text's internal font
  /// storage is opaque and is measured separately at the process level.
  var estimatedBytesForTesting: Int {
    lock.lock(); defer { lock.unlock() }
    return resources.values.reduce(0) { total, resource in
      total + 128 + resource.signature.count + resource.batches.reduce(0) {
        $0 + 64 + $1.glyphs.count * MemoryLayout<CGGlyph>.stride
          + $1.positions.count * MemoryLayout<CGPoint>.stride
      }
    }
  }

  func retain(identifiers: [UInt64], generation: UInt64) -> RenderResourceLease? {
    let unique = Array(Set(identifiers))
    lock.lock()
    guard self.generation == generation && unique.allSatisfy({ resources[$0] != nil }) else {
      lock.unlock(); return nil
    }
    for identifier in unique { references[identifier, default: 0] += 1 }
    lock.unlock()
    return RenderResourceLease(registry: self, identifiers: unique, generation: generation)
  }

  func releaseResponseResources(identifiers: [UInt64], generation: UInt64) {
    release(identifiers: identifiers, generation: generation)
  }

  fileprivate func release(identifiers: [UInt64], generation: UInt64) {
    var retired: [Resource] = []
    lock.lock()
    if self.generation == generation {
      for identifier in identifiers {
        guard let count = references[identifier] else { continue }
        if count > 1 { references[identifier] = count - 1 }
        else {
          references.removeValue(forKey: identifier)
          if let resource = resources.removeValue(forKey: identifier) { retired.append(resource) }
        }
      }
      // Dictionary removal preserves its largest bucket allocation. A rare
      // full-layout request must not permanently size this viewport registry.
      if resources.isEmpty {
        resources.removeAll(keepingCapacity: false)
        references.removeAll(keepingCapacity: false)
      } else {
        if resources.capacity > max(256, resources.count * 4) {
          resources = Dictionary(uniqueKeysWithValues: resources.lazy.map { ($0.key, $0.value) })
        }
        if references.capacity > max(256, references.count * 4) {
          references = Dictionary(uniqueKeysWithValues: references.lazy.map { ($0.key, $0.value) })
        }
      }
    }
    lock.unlock()
    Self.releaseNativeResources(retired)
  }

  private static func releaseNativeResources(_ retired: [Resource]) {
    if !Thread.isMainThread && !retired.isEmpty {
      // The closure holds the final registry reference until the native render
      // executor can release fonts and glyph arrays. Never block a core worker.
      DispatchQueue.main.async { withExtendedLifetime(retired) {} }
    }
  }

  init(generation: UInt64) {
    self.generation = generation
  }

  /// Retires every native object from older layout snapshots. Call this only
  /// when the provider's advertised metrics generation changes, or when its
  /// owning view is detached from core.
  public func retireAll(forNewGeneration newGeneration: UInt64) {
    lock.lock()
    generation = newGeneration
    let retired = Array(resources.values)
    resources.removeAll(keepingCapacity: false)
    references.removeAll(keepingCapacity: false)
    lock.unlock()
    Self.releaseNativeResources(retired)
  }

  public func contains(identifier: UInt64, metricsGeneration: UInt64) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return generation == metricsGeneration && resources[identifier] != nil
  }

  /// The immutable resolved font supplies inherited marker typography without
  /// measuring source text again or changing its layout. Stale generations
  /// never expose a resource from another measurement environment revision.
  public func resolvedFont(identifier: UInt64, metricsGeneration: UInt64) -> CTFont? {
    lock.lock()
    defer { lock.unlock() }
    return generation == metricsGeneration ? resources[identifier]?.batches.first?.font : nil
  }

  /// Read-only typography from the exact retained render resource. Markers
  /// inherit this without resolving styles again or remeasuring source text.
  public func textAttributes(identifier: UInt64, metricsGeneration: UInt64) -> CoreTextRenderAttributes? {
    lock.lock()
    defer { lock.unlock() }
    return generation == metricsGeneration ? resources[identifier]?.textAttributes : nil
  }

  /// Test visibility into the actual font retained for a displayed glyph run.
  /// This intentionally checks native draw resources, not the requested family.
  func resolvedFontFamily(identifier: UInt64, metricsGeneration: UInt64) -> String? {
    lock.lock()
    let font = generation == metricsGeneration ? resources[identifier]?.batches.first?.font : nil
    lock.unlock()
    return font.map { CTFontCopyFamilyName($0) as String }
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
    let font =
      generation == metricsGeneration
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
    let font =
      generation == metricsGeneration
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

  /// An en is half the current em; unlike a space it is not font-dependent
  /// word spacing and remains useful on an empty line.
  public func enAdvance(identifier: UInt64, metricsGeneration: UInt64) -> CGFloat? {
    lock.lock()
    let font = generation == metricsGeneration ? resources[identifier]?.batches.first?.font : nil
    lock.unlock()
    guard let font else { return nil }
    return CTFontGetSize(font) / 2
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
    draw(
      identifier: identifier,
      metricsGeneration: metricsGeneration,
      atBaseline: baseline,
      color: color.cgColor,
      in: context,
      clip: clip
    )
  }

  /// Accepts a paint color already resolved by the caller, so a viewport can
  /// reuse one native color across every cluster in the same paint run.
  @discardableResult
  public func draw(
    identifier: UInt64,
    metricsGeneration: UInt64,
    atBaseline baseline: CGPoint,
    color: CGColor,
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
    context.setFillColor(color)
    context.setTextDrawingMode(.fill)
    context.textMatrix = .identity
    context.translateBy(x: baseline.x, y: baseline.y)
    context.scaleBy(x: 1, y: -1)

    for batch in resource.batches {
      if batch.strokeWidth > 0 {
        context.setLineWidth(batch.strokeWidth)
        context.setStrokeColor(color)
        context.setTextDrawingMode(.fillStroke)
      } else {
        context.setTextDrawingMode(.fill)
      }
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

  func install(_ resource: Resource, preferredIdentifier: UInt64, generation: UInt64,
    pin: Bool = false) -> UInt64? {
    lock.lock()
    defer { lock.unlock() }
    guard self.generation == generation else { return nil }

    var identifier = preferredIdentifier == 0 ? 1 : preferredIdentifier
    while let existing = resources[identifier] {
      if existing.signature == resource.signature {
        if pin { references[identifier, default: 0] += 1 }
        return identifier
      }
      identifier = identifier &+ 1
      if identifier == 0 { identifier = 1 }
    }
    resources[identifier] = resource
    if pin { references[identifier, default: 0] += 1 }
    return identifier
  }
}

/// Independent of the provider/view lifetime. Rust shares one of these leases
/// across all clusters in a fragment and releases it after its final snapshot.
final class RenderResourceLease: @unchecked Sendable {
  let registry: CoreTextRenderRegistry
  let identifiers: [UInt64]
  let generation: UInt64
  init(registry: CoreTextRenderRegistry, identifiers: [UInt64], generation: UInt64) {
    self.registry = registry; self.identifiers = identifiers; self.generation = generation
  }
  deinit { registry.release(identifiers: identifiers, generation: generation) }
}
