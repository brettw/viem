import AppKit
import CViemCore
import CoreGraphics
import CoreText
import Foundation
import Testing

@testable import ViemCoreTextProvider

@Suite("Core Text measurement provider")
struct CoreTextMeasurementProviderTests {
  @Test("Tracking preserves default ligatures and explicit font feature choices")
  func trackingPreservesLigatures() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 84)
    for spacing: Float in [0, 2, -0.5] {
      let implicit = try shape(provider: provider, text: "fi", globalStart: 0,
        fontFamily: "Avenir Next", letterSpacing: spacing)
      #expect(implicit.clusters.count == 1)
      for enabled: UInt32 in [0, 1] {
        let explicit = try shape(provider: provider, text: "fi", globalStart: 0,
          openTypeFeature: (tag: (108, 105, 103, 97), value: enabled),
          fontFamily: "Avenir Next", letterSpacing: spacing)
        #expect(explicit.clusters.count == (enabled == 1 ? 1 : 2))
      }
    }
  }

  @Test("Pair kerning is implicit and tracking preserves native caret metrics at every scale")
  func implicitKerningAndTracking() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 83)
    for scale: Float in [1, 2] {
      let font = resolveFont(
        families: ["Times New Roman"], size: CGFloat(24 * scale),
        cssWeight: 400, slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
      let native = CTLineCreateWithAttributedString(
        NSAttributedString(string: "AV", attributes: [.font: font]))
      let disabled = CTLineCreateWithAttributedString(
        NSAttributedString(string: "AV", attributes: [.font: font, .kern: 0]))
      #expect(
        CTLineGetTypographicBounds(native, nil, nil, nil)
          < CTLineGetTypographicBounds(disabled, nil, nil, nil))
      for spacing: Float in [0, 2, -0.5] {
        let reference = CTLineCreateWithAttributedString(
          NSAttributedString(
            string: "AV",
            attributes: [.font: font, .tracking: CGFloat(spacing * scale)]))
        // Tracking's final space lies beyond Core Text's terminal caret. The
        // editor uses the native caret extent for placement and block geometry.
        let expected = CTLineGetOffsetForStringIndex(reference, 2, nil)
        let measured = try shape(
          provider: provider, text: "AV", globalStart: 0,
          fontFamily: "Times New Roman", fontSize: 24, letterSpacing: spacing, scale: scale)
        let rendered = try shape(
          provider: provider, text: "AV", globalStart: 0,
          purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
          openTypeFeature: (tag: (107, 101, 114, 110), value: 0),
          fontFamily: "Times New Roman", fontSize: 24, letterSpacing: spacing, scale: scale)
        #expect(abs(measured.clusters.reduce(0.0) { $0 + Double($1.advance) } - expected) < 0.001)
        #expect(measured.clusters.map(\.advance) == rendered.clusters.map(\.advance))
        #expect(rendered.clusters.allSatisfy { $0.hasRenderRun == 1 })
        let contextual = try shape(
          provider: provider, text: "A", globalStart: 0, contextAfter: "V",
          fontFamily: "Times New Roman", fontSize: 24, letterSpacing: spacing, scale: scale)
        #expect(contextual.clusters[0].advance == measured.clusters[0].advance)
      }
    }
  }

  @Test("Arabic and mixed bidi cluster advances equal the native shaped line")
  func arabicClusterMetricsMatchNativeLine() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 81)
    let font = resolveFont(
      families: ["SF Pro"], size: 14, cssWeight: 400,
      slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
    for text in [
      "Arabic word مرحبا in the middle", "مرحبا بالعالم", "English العربية 123 ثم English",
      "سَلامٌ e\u{301} 👩🏽‍💻 مرحبا",
    ] {
      let shaped = try shape(provider: provider, text: text, globalStart: 0)
      let attributed = NSAttributedString(
        string: text,
        attributes: [
          NSAttributedString.Key(kCTFontAttributeName as String): font
        ])
      let native = CTLineCreateWithAttributedString(attributed)
      let width = CTLineGetTypographicBounds(native, nil, nil, nil)
      let total = shaped.clusters.reduce(0.0) { $0 + Double($1.advance) }
      #expect(abs(total - width) < 0.001, "\(text): provider \(total), native \(width)")
    }
  }
  @Test("Explicit font families resolve bold and italic faces")
  func explicitFamilyTraits() {
    for family in ["Helvetica", "Times New Roman", "SF Pro"] {
      let normal = resolveFont(
        families: [family], size: 14, cssWeight: 400,
        slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
      let bold = resolveFont(
        families: [family], size: 14, cssWeight: 700,
        slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
      let italic = resolveFont(
        families: [family], size: 14, cssWeight: 400,
        slant: UInt32(VIEM_FONT_SLANT_ITALIC), features: [])
      let both = resolveFont(
        families: [family], size: 14, cssWeight: 700,
        slant: UInt32(VIEM_FONT_SLANT_ITALIC), features: [])
      #expect(!CTFontGetSymbolicTraits(normal).contains(.traitBold))
      #expect(!CTFontGetSymbolicTraits(normal).contains(.traitItalic))
      #expect(CTFontGetSymbolicTraits(bold).contains(.traitBold))
      #expect(CTFontGetSymbolicTraits(italic).contains(.traitItalic))
      #expect(CTFontGetSymbolicTraits(both).contains([.traitBold, .traitItalic]))
      #expect(CTFontCopyPostScriptName(normal) != CTFontCopyPostScriptName(bold))
    }
  }

  @Test("SF Pro 14 is the declared frontend default")
  func declaredDefault() {
    #expect(CoreTextMeasurementProvider.defaultFontFamily == "SF Pro")
    #expect(CoreTextMeasurementProvider.defaultFontSize == 14)
  }

  @Test("ASCII shaping is proportional and owns global UTF-8 ranges")
  func proportionalASCII() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 41)
    let result = try shape(
      provider: provider,
      text: "im",
      globalStart: 100,
      purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
    )

    #expect(result.response.text_start == 100)
    #expect(result.response.text_end == 102)
    #expect(result.clusters.first?.textStart == 100)
    #expect(result.clusters.last?.textEnd == 102)
    #expect(result.clusters.count == 2)
    #expect(result.clusters[0].advance != result.clusters[1].advance)
    #expect(result.response.default_metrics.ascent > 0)
    #expect(result.clusters.allSatisfy { $0.hasRenderRun == 1 })
    #expect(
      result.clusters.allSatisfy {
        provider.renderRegistry.contains(
          identifier: $0.renderRun.identifier,
          metricsGeneration: $0.renderRun.metrics_generation
        )
      })

    let handle = result.clusters[0].renderRun
    let spaceAdvance = try #require(
      provider.renderRegistry.spaceAdvance(
        identifier: handle.identifier,
        metricsGeneration: handle.metrics_generation
      ))
    #expect(spaceAdvance > 0)
    let bitmap = try #require(
      CGContext(
        data: nil,
        width: 80,
        height: 40,
        bitsPerComponent: 8,
        bytesPerRow: 80 * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
      ))
    #expect(
      provider.renderRegistry.draw(
        identifier: handle.identifier,
        metricsGeneration: handle.metrics_generation,
        atBaseline: CGPoint(x: 5, y: 25),
        color: .textColor,
        in: bitmap
      ))
    let pixels = try #require(bitmap.data).assumingMemoryBound(to: UInt8.self)
    #expect((0..<(80 * 40 * 4)).contains { pixels[$0] != 0 })
  }

  @Test("OpenType requests reach the resolved Core Text font")
  func openTypeFeatureRequest() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 49)
    let result = try shape(
      provider: provider,
      text: "office",
      globalStart: 0,
      purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
      openTypeFeature: (tag: (0x6C, 0x69, 0x67, 0x61), value: 0)
    )
    let handle = try #require(result.clusters.first?.renderRun)
    let settings = try #require(
      provider.renderRegistry.openTypeFeatureSettings(
        identifier: handle.identifier,
        metricsGeneration: handle.metrics_generation
      ))

    #expect(settings.contains { $0.tag == "liga" && $0.value == 0 })
  }

  @Test("clusters and caret stops remain on extended-grapheme UTF-8 boundaries")
  func graphemeSafeUTF8Offsets() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 42)
    let text = "A👩🏽‍💻e\u{301}ב"
    let start: UInt64 = 17
    let result = try shape(provider: provider, text: text, globalStart: start)
    let legal = Set(
      text.indices.map {
        start
          + UInt64(
            text.utf8.distance(from: text.utf8.startIndex, to: $0.samePosition(in: text.utf8)!))
      } + [start + UInt64(text.utf8.count)])

    #expect(result.clusters.first?.textStart == start)
    #expect(result.clusters.last?.textEnd == start + UInt64(text.utf8.count))
    for cluster in result.clusters {
      #expect(legal.contains(cluster.textStart))
      #expect(legal.contains(cluster.textEnd))
      #expect(cluster.caretStopCount >= 2)
      for caret in cluster.carets {
        #expect(legal.contains(caret.text_offset))
        #expect(caret.inline_offset >= 0)
        #expect(caret.inline_offset <= cluster.advance)
      }
    }
  }

  @Test("mixed style runs affect metrics without changing byte ownership")
  func mixedStyleRuns() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 43)
    let result = try shape(
      provider: provider,
      text: "ab",
      globalStart: 0,
      styleRun: (range: 1..<2, size: 28)
    )

    #expect(result.clusters.count == 2)
    #expect(result.clusters[1].metrics.ascent > result.clusters[0].metrics.ascent * 1.7)
    #expect(result.clusters[0].textStart == 0)
    #expect(result.clusters[1].textEnd == 2)
  }

  @Test("metrics-only requests never retain native render resources")
  func metricsOnly() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 44)
    let result = try shape(
      provider: provider,
      text: "metrics",
      globalStart: 0,
      purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_ONLY)
    )
    #expect(result.clusters.allSatisfy { $0.hasRenderRun == 0 })
  }

  @Test("Fragment leases bound native resources and survive response replacement")
  func renderResourceLeases() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 145)
    let table = provider.makeProviderTable()
    let retain = try #require(table.retain_render_runs)
    let release = try #require(table.release_render_runs)
    var kept: [(lease: UnsafeMutableRawPointer, handles: [ViemRenderRunHandleV1])] = []
    defer { for item in kept { release(item.lease) } }
    for index in 0..<512 {
      let result = try shape(provider: provider, text: "lease \(index) 👩🏽‍💻",
        globalStart: UInt64(index * 64), purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
        fontSize: Float(10 + index % 24))
      let handles = result.clusters.filter { $0.hasRenderRun == 1 }.map(\.renderRun)
      let lease = try #require(handles.withUnsafeBufferPointer {
        retain(table.context, $0.baseAddress, UInt64($0.count))
      })
      kept.append((lease, handles))
      if kept.count > 4 { release(kept.removeFirst().lease) }
      #expect(provider.renderRegistry.resourceCountForTesting < 128)
      #expect(provider.renderRegistry.estimatedBytesForTesting < 128 * 1024)
      for item in kept {
        #expect(item.handles.allSatisfy { provider.renderRegistry.contains(
          identifier: $0.identifier, metricsGeneration: $0.metrics_generation) })
      }
    }
    // End the last response lifetime while keeping the four snapshot leases.
    _ = try shape(provider: provider, text: "metrics only", globalStart: 0)
    #expect(provider.renderRegistry.resourceCountForTesting > 0)
    for item in kept { release(item.lease) }; kept.removeAll()
    #expect(provider.renderRegistry.resourceCountForTesting == 0)
    #expect(provider.renderRegistry.estimatedBytesForTesting == 0)
  }

  @Test("changing metrics generation retires render handles")
  func generationLifetime() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 45)
    let result = try shape(
      provider: provider,
      text: "x",
      globalStart: 0,
      purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
    )
    let handle = try #require(result.clusters.first?.renderRun)
    #expect(
      provider.renderRegistry.contains(
        identifier: handle.identifier,
        metricsGeneration: handle.metrics_generation
      ))

    let newGeneration = provider.invalidateMetrics()
    #expect(newGeneration != handle.metrics_generation)
    #expect(
      !provider.renderRegistry.contains(
        identifier: handle.identifier,
        metricsGeneration: handle.metrics_generation
      ))
  }

  @Test("detached native leases release on a worker after their provider is gone")
  func detachedLeaseOutlivesProvider() throws {
    var provider: CoreTextMeasurementProvider? = CoreTextMeasurementProvider(measurementEnvironmentID: 146)
    weak let weakProvider = provider
    weak let weakRegistry = provider?.renderRegistry
    let table = try #require(provider).makeProviderTable()
    let retain = try #require(table.retain_render_runs)
    let release = try #require(table.release_render_runs)
    let result = try shape(provider: #require(provider), text: "detached 👩🏽‍💻", globalStart: 0,
      purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA))
    let handles = result.clusters.filter { $0.hasRenderRun == 1 }.map(\.renderRun)
    let lease = try #require(handles.withUnsafeBufferPointer {
      retain(table.context, $0.baseAddress, UInt64($0.count))
    })
    provider?.retireResources()
    #expect(provider?.retainedResponseArenaCountForTesting == 0)
    #expect(weakRegistry?.resourceCountForTesting == 0)
    provider = nil
    #expect(weakProvider == nil)
    #expect(weakRegistry != nil) // The independent lease alone owns the registry.
    let released = DispatchSemaphore(value: 0)
    let address = UInt(bitPattern: lease)
    DispatchQueue.global(qos: .userInitiated).async {
      release(UnsafeMutableRawPointer(bitPattern: address))
      released.signal()
    }
    #expect(released.wait(timeout: .now() + 2) == .success)
    #expect(weakRegistry == nil)
  }

  @Test("late pre-detach leases cannot release a reused provider's new resources")
  func detachedLeaseCannotRetireNewGeneration() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 147)
    let table = provider.makeProviderTable()
    let retain = try #require(table.retain_render_runs)
    let release = try #require(table.release_render_runs)
    func retainedShape() throws -> (UnsafeMutableRawPointer, [ViemRenderRunHandleV1]) {
      let result = try shape(provider: provider, text: "same glyphs", globalStart: 0,
        purpose: UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA))
      let handles = result.clusters.filter { $0.hasRenderRun == 1 }.map(\.renderRun)
      return (try #require(handles.withUnsafeBufferPointer {
        retain(table.context, $0.baseAddress, UInt64($0.count))
      }), handles)
    }
    let (oldLease, oldHandles) = try retainedShape()
    provider.retireResources()
    let (newLease, newHandles) = try retainedShape()
    defer { release(newLease) }
    #expect(oldHandles[0].metrics_generation != newHandles[0].metrics_generation)
    // End the new response pins. Only the new fragment lease now owns these.
    _ = try shape(provider: provider, text: "metrics only", globalStart: 0)
    let released = DispatchSemaphore(value: 0)
    let address = UInt(bitPattern: oldLease)
    DispatchQueue.global(qos: .userInitiated).async {
      release(UnsafeMutableRawPointer(bitPattern: address))
      released.signal()
    }
    #expect(released.wait(timeout: .now() + 2) == .success)
    #expect(newHandles.allSatisfy { provider.renderRegistry.contains(
      identifier: $0.identifier, metricsGeneration: $0.metrics_generation) })
  }

  @Test("asynchronous metrics invalidation retains the current response storage")
  func invalidationPreservesResponseLifetime() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 47)
    let result = try shape(provider: provider, text: "retain 👩🏽‍💻", globalStart: 31)
    let clusters = try #require(result.response.clusters)
    let firstStart = clusters.pointee.text_start
    let firstFallback = try decoded(clusters.pointee.fallback_font)
    #expect(provider.retainedResponseArenaCountForTesting == 1)

    let invalidated = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .userInitiated).async {
      _ = provider.invalidateMetrics()
      invalidated.signal()
    }
    #expect(invalidated.wait(timeout: .now() + 2) == .success)

    // Invalidation is not itself a provider callback, so every pointer from the
    // preceding response must remain owned and readable.
    #expect(provider.retainedResponseArenaCountForTesting == 1)
    #expect(clusters.pointee.text_start == firstStart)
    #expect(try decoded(clusters.pointee.fallback_font) == firstFallback)

    provider.retireResources()
    #expect(provider.retainedResponseArenaCountForTesting == 0)
  }

  @Test("metrics invalidation does not wait for Core Text shaping")
  func invalidationDoesNotWaitForShaping() {
    let shapingStarted = DispatchSemaphore(value: 0)
    let resumeShaping = DispatchSemaphore(value: 0)
    let provider = CoreTextMeasurementProvider(
      measurementEnvironmentID: 48,
      initialMetricsGeneration: 1,
      shapingDidBegin: {
        shapingStarted.signal()
        _ = resumeShaping.wait(timeout: .now() + 5)
      }
    )
    let shapeFinished = DispatchSemaphore(value: 0)
    let shapeStatus = LockedBox<UInt32?>(nil)

    DispatchQueue.global(qos: .userInitiated).async {
      defer { shapeFinished.signal() }
      do {
        _ = try shape(
          provider: provider,
          text: "Core Text runs outside the provider state lock",
          globalStart: 0,
          expectedStatus: nil
        )
        shapeStatus.set(0)
      } catch let ShapeTestError.status(status) {
        shapeStatus.set(status)
      } catch {
        shapeStatus.set(UInt32.max)
      }
    }

    let didStart = shapingStarted.wait(timeout: .now() + 2) == .success
    #expect(didStart)
    guard didStart else {
      resumeShaping.signal()
      return
    }

    let invalidationFinished = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .userInitiated).async {
      _ = provider.invalidateMetrics()
      invalidationFinished.signal()
    }
    let invalidatedWhileShapePaused = invalidationFinished.wait(timeout: .now() + 1) == .success
    resumeShaping.signal()
    let didFinish = shapeFinished.wait(timeout: .now() + 5) == .success

    #expect(invalidatedWhileShapePaused)
    #expect(didFinish)
    #expect(shapeStatus.value == 23)
  }

  @Test("bounded context partitions cross-fragment shaping by logical cluster start")
  func boundedContextOwnership() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 46)
    let head = try shape(
      provider: provider,
      text: "f",
      globalStart: 0,
      contextAfter: "i"
    )
    let tail = try shape(
      provider: provider,
      text: "i",
      globalStart: 1,
      contextBefore: "f"
    )

    let combined = (head.clusters + tail.clusters).sorted { $0.textStart < $1.textStart }
    #expect(combined.first?.textStart == 0)
    #expect(combined.last?.textEnd == 2)
    for pair in zip(combined, combined.dropFirst()) {
      #expect(pair.0.textEnd == pair.1.textStart)
    }
  }

  @Test("nested directional embeddings retain their resolved UAX levels")
  func nestedDirectionalEmbeddings() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 50)
    let text = "A\u{202B}אב\u{202A}12\u{202C}גד\u{202C}Z"
    let globalStart: UInt64 = 100
    let result = try shape(provider: provider, text: text, globalStart: globalStart)

    let outerRTL = try cluster(containing: "א", in: text, result: result, globalStart: globalStart)
    let nestedLTR = try cluster(containing: "1", in: text, result: result, globalStart: globalStart)
    let paragraphLTR = try cluster(
      containing: "Z", in: text, result: result, globalStart: globalStart)
    #expect(outerRTL.bidiLevel == 1)
    #expect(nestedLTR.bidiLevel == 2)
    #expect(paragraphLTR.bidiLevel == 0)

    let outerIndex = try #require(result.clusters.firstIndex { $0.textStart == outerRTL.textStart })
    let nestedIndex = try #require(
      result.clusters.firstIndex { $0.textStart == nestedLTR.textStart })
    let trailingRTL = try cluster(
      containing: "ג", in: text, result: result, globalStart: globalStart)
    let trailingIndex = try #require(
      result.clusters.firstIndex { $0.textStart == trailingRTL.textStart })
    let outerVisualIndex = try #require(result.visualOrder.firstIndex(of: outerIndex))
    let nestedVisualIndex = try #require(result.visualOrder.firstIndex(of: nestedIndex))
    let trailingVisualIndex = try #require(result.visualOrder.firstIndex(of: trailingIndex))
    #expect(trailingVisualIndex < nestedVisualIndex)
    #expect(nestedVisualIndex < outerVisualIndex)
  }

  @Test("nested directional isolates retain levels beyond the paragraph heuristic")
  func nestedDirectionalIsolates() throws {
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 51)
    let text = "אב \u{2066}abc \u{2067}גד\u{2069} xyz\u{2069} ה"
    let globalStart: UInt64 = 200
    let result = try shape(provider: provider, text: text, globalStart: globalStart)

    let paragraphRTL = try cluster(
      containing: "א", in: text, result: result, globalStart: globalStart)
    let isolatedLTR = try cluster(
      containing: "a", in: text, result: result, globalStart: globalStart)
    let nestedRTL = try cluster(
      containing: "ג", in: text, result: result, globalStart: globalStart)
    #expect(paragraphRTL.bidiLevel == 1)
    #expect(isolatedLTR.bidiLevel == 2)
    #expect(nestedRTL.bidiLevel == 3)
    #expect(result.clusters.map(\.bidiLevel).max() == 3)
  }
}

private struct OwnedCluster {
  let value: ViemShapedClusterV1
  let carets: [ViemClusterCaretStopV1]

  var textStart: UInt64 { value.text_start }
  var textEnd: UInt64 { value.text_end }
  var advance: Float { value.advance }
  var metrics: ViemTextMetricsV1 { value.metrics }
  var bidiLevel: UInt32 { value.bidi_level }
  var caretStopCount: UInt64 { value.caret_stop_count }
  var hasRenderRun: UInt32 { value.has_render_run }
  var renderRun: ViemRenderRunHandleV1 { value.render_run }
}

private struct ShapeResult {
  let response: ViemShapeResponseV1
  let clusters: [OwnedCluster]
  let visualOrder: [Int]
}

private func shape(
  provider: CoreTextMeasurementProvider,
  text: String,
  globalStart: UInt64,
  contextBefore: String = "",
  contextAfter: String = "",
  purpose: UInt32 = UInt32(VIEM_SHAPE_PURPOSE_METRICS_ONLY),
  styleRun: (range: Range<UInt64>, size: Float)? = nil,
  openTypeFeature: (tag: (UInt8, UInt8, UInt8, UInt8), value: UInt32)? = nil,
  fontFamily: String = "SF Pro",
  fontSize: Float = 14,
  letterSpacing: Float = 0,
  scale: Float = 1,
  expectedStatus: UInt32? = 0
) throws -> ShapeResult {
  let table = provider.makeProviderTable()
  let callback = try #require(table.shape_batch)
  let textBytes = Array(text.utf8)
  let beforeBytes = Array(contextBefore.utf8)
  let afterBytes = Array(contextAfter.utf8)
  let familyBytes = Array(fontFamily.utf8)
  var featureValues: [ViemOpenTypeFeatureV1] = []
  if let openTypeFeature {
    var feature = ViemOpenTypeFeatureV1()
    feature.tag = openTypeFeature.tag
    feature.value = openTypeFeature.value
    featureValues.append(feature)
  }
  var response = ViemShapeResponseV1()

  let status: UInt32 = textBytes.withUnsafeBufferPointer { textBuffer in
    beforeBytes.withUnsafeBufferPointer { beforeBuffer in
      afterBytes.withUnsafeBufferPointer { afterBuffer in
        familyBytes.withUnsafeBufferPointer { familyBuffer in
          featureValues.withUnsafeBufferPointer { featureBuffer in
            var family = ViemUtf8Slice()
            family.data = familyBuffer.baseAddress
            family.length = UInt64(familyBuffer.count)

            var style = ViemResolvedTextStyleV1()
            style.struct_size = UInt32(MemoryLayout<ViemResolvedTextStyleV1>.size)
            style.slant = UInt32(VIEM_FONT_SLANT_UPRIGHT)
            style.direction = UInt32(VIEM_TEXT_DIRECTION_AUTO)
            style.size = fontSize
            style.weight = 400
            style.letter_spacing = letterSpacing
            style.font_family_count = 1
            style.font_families = withUnsafePointer(to: &family) { $0 }
            style.features = featureBuffer.baseAddress
            style.feature_count = UInt64(featureBuffer.count)

            var request = ViemShapeRequestV1()
            request.struct_size = UInt32(MemoryLayout<ViemShapeRequestV1>.size)
            request.purpose = purpose
            request.document_id = 7
            request.document_revision = 9
            request.measurement_environment_id = provider.measurementEnvironmentID
            request.metrics_generation = provider.metricsGeneration
            request.text_start = globalStart
            request.text_end = globalStart + UInt64(textBuffer.count)
            request.text.data = textBuffer.baseAddress
            request.text.length = UInt64(textBuffer.count)
            request.context_before.data = beforeBuffer.baseAddress
            request.context_before.length = UInt64(beforeBuffer.count)
            request.context_after.data = afterBuffer.baseAddress
            request.context_after.length = UInt64(afterBuffer.count)
            request.default_style = style
            request.scale = scale
            request.paragraph_base_direction = UInt32(VIEM_TEXT_DIRECTION_AUTO)
            if purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA) {
              request.has_render_run_policy = 1
              request.render_run_owner = provider.renderRunOwner
              request.render_run_threading = UInt32(VIEM_RENDER_THREADING_FRONTEND_MAIN)
            }

            if let styleRun {
              var largeStyle = style
              largeStyle.size = styleRun.size
              var run = ViemShapeStyleRunV1()
              run.struct_size = UInt32(MemoryLayout<ViemShapeStyleRunV1>.size)
              run.text_start = styleRun.range.lowerBound
              run.text_end = styleRun.range.upperBound
              run.style = largeStyle
              return withUnsafePointer(to: &family) { familyPointer in
                style.font_families = familyPointer
                request.default_style = style
                largeStyle.font_families = familyPointer
                run.style = largeStyle
                return withUnsafePointer(to: &run) { runPointer in
                  request.style_runs = runPointer
                  request.style_run_count = 1
                  return withUnsafePointer(to: &request) { requestPointer in
                    withUnsafeMutablePointer(to: &response) { responsePointer in
                      callback(table.context, requestPointer, 1, responsePointer, 1)
                    }
                  }
                }
              }
            }

            return withUnsafePointer(to: &family) { familyPointer in
              style.font_families = familyPointer
              request.default_style = style
              return withUnsafePointer(to: &request) { requestPointer in
                withUnsafeMutablePointer(to: &response) { responsePointer in
                  callback(table.context, requestPointer, 1, responsePointer, 1)
                }
              }
            }
          }
        }
      }
    }
  }
  if let expectedStatus {
    #expect(status == expectedStatus)
  }
  guard status == 0 else { throw ShapeTestError.status(status) }

  let clusterValues: [OwnedCluster]
  if response.cluster_count == 0 {
    clusterValues = []
  } else {
    let pointer = try #require(response.clusters)
    clusterValues = (0..<Int(response.cluster_count)).map { index in
      let value = pointer[index]
      let carets =
        value.caret_stop_count == 0
        ? []
        : Array(UnsafeBufferPointer(start: value.caret_stops!, count: Int(value.caret_stop_count)))
      return OwnedCluster(value: value, carets: carets)
    }
  }
  let visualOrder: [Int]
  if response.visual_order_count == 0 {
    visualOrder = []
  } else {
    let pointer = try #require(response.visual_order)
    visualOrder = Array(
      UnsafeBufferPointer(start: pointer, count: Int(response.visual_order_count))
    ).map(Int.init)
  }
  return ShapeResult(response: response, clusters: clusterValues, visualOrder: visualOrder)
}

private func cluster(
  containing needle: String,
  in text: String,
  result: ShapeResult,
  globalStart: UInt64
) throws -> OwnedCluster {
  let range = try #require(text.range(of: needle))
  let localOffset = UInt64(text[..<range.lowerBound].utf8.count)
  let offset = globalStart + localOffset
  return try #require(
    result.clusters.first { $0.textStart <= offset && offset < $0.textEnd }
  )
}

private enum ShapeTestError: Error {
  case status(UInt32)
}

private func decoded(_ slice: ViemUtf8Slice) throws -> String {
  guard slice.length <= UInt64(Int.max), let data = slice.data else {
    throw ShapeTestError.status(UInt32(VIEM_STATUS_INVALID_ARGUMENT))
  }
  return String(
    decoding: UnsafeBufferPointer(start: data, count: Int(slice.length)),
    as: UTF8.self
  )
}

private final class LockedBox<Value>: @unchecked Sendable {
  private let lock = NSLock()
  private var stored: Value

  init(_ value: Value) {
    stored = value
  }

  func set(_ value: Value) {
    lock.lock()
    stored = value
    lock.unlock()
  }

  var value: Value {
    lock.lock()
    defer { lock.unlock() }
    return stored
  }
}
