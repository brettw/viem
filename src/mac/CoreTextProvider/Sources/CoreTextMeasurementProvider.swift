import AppKit
import CViemCore
import CoreFoundation
import CoreGraphics
import CoreText
import Foundation

/// Core Text implementation of Viem's batched measurement ABI.
///
/// Core Text font and shaping operations are safe off the main thread, so the
/// provider advertises worker execution. Opaque draw resources remain confined
/// to the frontend main thread through their separate render policy.
public final class CoreTextMeasurementProvider: @unchecked Sendable {
  public static let defaultFontFamily = "SF Pro"
  public static let defaultFontSize: CGFloat = 14

  public let measurementEnvironmentID: UInt64
  public let renderRunOwner: UInt64
  public let renderRegistry: CoreTextRenderRegistry

  // Protects only generation publication and response-arena ownership. Core
  // Text work deliberately runs outside this lock: font resolution may
  // synchronously announce a metrics change, and duplicate shaping is cheaper
  // than blocking invalidation behind an expensive batch.
  private let stateLock = NSLock()
  private var shapeBatchCalls: UInt64 = 0
  var shapeBatchCallCount: UInt64 {
    stateLock.lock()
    defer { stateLock.unlock() }
    return shapeBatchCalls
  }
  private var generation: UInt64
  private var responseArenas: [ResponseArena] = []
  private var localFontObserver: NSObjectProtocol?
  private var distributedFontObserver: NSObjectProtocol?
  private let shapingDidBegin: (@Sendable () -> Void)?

  public convenience init(
    measurementEnvironmentID: UInt64 = UInt64.random(in: 1...UInt64.max),
    initialMetricsGeneration: UInt64 = 1
  ) {
    self.init(
      measurementEnvironmentID: measurementEnvironmentID,
      initialMetricsGeneration: initialMetricsGeneration,
      shapingDidBegin: nil
    )
  }

  init(
    measurementEnvironmentID: UInt64,
    initialMetricsGeneration: UInt64,
    shapingDidBegin: (@Sendable () -> Void)?
  ) {
    self.measurementEnvironmentID = measurementEnvironmentID
    let proposedOwner = measurementEnvironmentID ^ 0x4354_5255_4E53_4554
    renderRunOwner = proposedOwner == 0 ? 0x5649_454D : proposedOwner
    generation = max(initialMetricsGeneration, 1)
    renderRegistry = CoreTextRenderRegistry(generation: max(initialMetricsGeneration, 1))
    self.shapingDidBegin = shapingDidBegin

    let notificationName = Notification.Name(
      kCTFontManagerRegisteredFontsChangedNotification as String)
    localFontObserver = NotificationCenter.default.addObserver(
      forName: notificationName,
      object: nil,
      queue: nil
    ) { [weak self] _ in
      self?.invalidateMetrics()
    }
    distributedFontObserver = DistributedNotificationCenter.default().addObserver(
      forName: notificationName,
      object: nil,
      queue: nil
    ) { [weak self] _ in
      self?.invalidateMetrics()
    }
  }

  deinit {
    if let localFontObserver {
      NotificationCenter.default.removeObserver(localFontObserver)
    }
    if let distributedFontObserver {
      DistributedNotificationCenter.default().removeObserver(distributedFontObserver)
    }
  }

  public var metricsGeneration: UInt64 {
    stateLock.lock()
    defer { stateLock.unlock() }
    return generation
  }

  /// Invalidates shaped metrics after font registration, backing-scale, or
  /// another resolver input changes. Existing opaque render handles are
  /// retired at the same generation boundary required by the ABI. Response
  /// storage is intentionally retained: asynchronous invalidation is not a
  /// provider callback and therefore cannot end the C ABI response lifetime.
  @discardableResult
  public func invalidateMetrics() -> UInt64 {
    EVFontCatalog.invalidate()
    stateLock.lock()
    generation = generation &+ 1
    if generation == 0 { generation = 1 }
    let current = generation
    renderRegistry.retireAll(forNewGeneration: current)
    stateLock.unlock()
    return current
  }

  /// Ends all response and render-resource lifetimes after a view has been
  /// removed from core. The provider object itself must still outlive the C
  /// provider table installed for that view.
  public func retireResources() {
    var retiredArenas: [ResponseArena] = []
    stateLock.lock()
    generation = generation &+ 1
    if generation == 0 { generation = 1 }
    retiredArenas = responseArenas
    responseArenas.removeAll(keepingCapacity: false)
    renderRegistry.retireAll(forNewGeneration: generation)
    stateLock.unlock()
    // Response allocations can be large; release them without holding the
    // provider state lock after the owning view has ended their ABI lifetime.
    retiredArenas.removeAll(keepingCapacity: false)
  }

  var retainedResponseArenaCountForTesting: Int {
    stateLock.lock()
    defer { stateLock.unlock() }
    return responseArenas.count
  }

  /// Produces the trivially-copyable table consumed by
  /// `viem_core_view_add`. The caller retains this provider until the view is
  /// removed or the owning core is successfully destroyed.
  public func makeProviderTable() -> ViemTextMeasurementProviderV1 {
    var table = ViemTextMeasurementProviderV1()
    table.struct_size = UInt32(MemoryLayout<ViemTextMeasurementProviderV1>.size)
    table.abi_version = UInt32(VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V3)
    table.context = Unmanaged.passUnretained(self).toOpaque()
    table.measurement_environment_id = measurementEnvironmentID
    table.threading = UInt32(VIEM_PROVIDER_THREADING_ANY_WORKER)
    table.has_render_run_policy = 1
    table.render_run_owner = renderRunOwner
    table.render_run_threading = UInt32(VIEM_RENDER_THREADING_FRONTEND_MAIN)
    table.reserved = 0
    table.metrics_generation = viemCoreTextMetricsGeneration
    table.shape_batch = viemCoreTextShapeBatch
    table.retain_render_runs = viemCoreTextRetainRenderRuns
    table.release_render_runs = viemCoreTextReleaseRenderRuns
    return table
  }

  fileprivate func shapeBatch(
    requests: UnsafePointer<ViemShapeRequestV1>?,
    requestCount: UInt64,
    responses: UnsafeMutablePointer<ViemShapeResponseV1>?,
    responseCapacity: UInt64
  ) -> UInt32 {
    guard requestCount <= responseCapacity,
      requestCount <= UInt64(Int.max),
      requestCount == 0 || (requests != nil && responses != nil)
    else { return Status.invalidArgument }

    // Beginning a new shape callback ends the previous shape callback's
    // response lifetime. Keep arenas from overlapping callbacks separate: an
    // earlier callback may still finish after this one has begun.
    var retiredArenas: [ResponseArena] = []
    let callbackGeneration: UInt64
    stateLock.lock()
    retiredArenas = responseArenas
    responseArenas.removeAll(keepingCapacity: true)
    callbackGeneration = generation
    shapeBatchCalls &+= 1
    stateLock.unlock()
    retiredArenas.removeAll(keepingCapacity: false)

    // Internal deterministic test instrumentation. Production providers leave
    // this nil; importantly, it runs in the same unlocked region as Core Text.
    shapingDidBegin?()

    let arena = ResponseArena(registry: renderRegistry, generation: callbackGeneration)
    var shaped: [ViemShapeResponseV1] = []
    shaped.reserveCapacity(Int(requestCount))

    for index in 0..<Int(requestCount) {
      do {
        let response = try shapeOne(
          requests![index],
          arena: arena,
          callbackGeneration: callbackGeneration
        )
        shaped.append(response)
      } catch let error as ProviderError {
        return error.status
      } catch {
        return Status.providerFailure
      }
    }

    stateLock.lock()
    defer { stateLock.unlock() }
    guard generation == callbackGeneration else { return Status.providerFailure }
    // Publish ownership before exposing the pointer-bearing responses. All
    // arenas completed by overlapping callbacks remain retained together until
    // a later callback boundary.
    responseArenas.append(arena)
    for (index, response) in shaped.enumerated() {
      responses![index] = response
    }
    return Status.ok
  }

  private func shapeOne(
    _ request: ViemShapeRequestV1,
    arena: ResponseArena,
    callbackGeneration: UInt64
  ) throws -> ViemShapeResponseV1 {
    guard request.struct_size >= UInt32(MemoryLayout<ViemShapeRequestV1>.size),
      request.measurement_environment_id == measurementEnvironmentID,
      request.metrics_generation == callbackGeneration,
      request.text_start <= request.text_end,
      request.scale.isFinite,
      request.scale > 0
    else { throw ProviderError(Status.invalidArgument) }

    let interior = try decode(request.text)
    let before = try decode(request.context_before)
    let after = try decode(request.context_after)
    guard UInt64(interior.utf8.count) == request.text_end - request.text_start,
      UInt64(before.utf8.count) <= request.text_start
    else { throw ProviderError(Status.invalidArgument) }

    let contextStart = request.text_start - UInt64(before.utf8.count)
    let fullText = before + interior + after
    guard
      let contextEnd = adding(contextStart, UInt64(fullText.utf8.count)),
      let interiorLocalEnd = adding(UInt64(before.utf8.count), UInt64(interior.utf8.count))
    else { throw ProviderError(Status.invalidArgument) }
    let indexMap = TextIndexMap(fullText)
    guard
      indexMap.utf16Offset(forUTF8: before.utf8.count) != nil,
      interiorLocalEnd <= UInt64(Int.max),
      indexMap.utf16Offset(forUTF8: Int(interiorLocalEnd)) != nil
    else { throw ProviderError(Status.invalidArgument) }
    let defaultStyle = try ResolvedStyle(request.default_style, scale: CGFloat(request.scale))
    let styleRuns = try readStyleRuns(request, scale: CGFloat(request.scale))
    let attributed = try makeAttributedString(
      fullText,
      contextStart: contextStart,
      indexMap: indexMap,
      defaultStyle: defaultStyle,
      styleRuns: styleRuns,
      paragraphDirection: request.paragraph_base_direction
    )
    let bidiLevels = resolvedBidiLevels(
      attributed,
      paragraphDirection: request.paragraph_base_direction
    )
    let line = CTLineCreateWithAttributedString(attributed)
    let glyphRecords = extractGlyphRecords(line)
    let glyphRecordsByClusterStart = Dictionary(grouping: glyphRecords) { record in
      indexMap.graphemeStart(containingUTF16: record.stringIndex)
    }
    let clusterBoundaries = clusterBoundaries(for: glyphRecords, map: indexMap)

    var clusters: [ClusterResult] = []
    var diagnostics: [DiagnosticResult] = []
    for boundaryIndex in 0..<max(clusterBoundaries.count - 1, 0) {
      let start16 = clusterBoundaries[boundaryIndex]
      let end16 = clusterBoundaries[boundaryIndex + 1]
      guard let localStart8 = indexMap.utf8Offset(forUTF16: start16),
        let localEnd8 = indexMap.utf8Offset(forUTF16: end16)
      else { throw ProviderError(Status.providerFailure) }
      guard
        let globalStart = adding(contextStart, UInt64(localStart8)),
        let globalEnd = adding(contextStart, UInt64(localEnd8)),
        globalEnd <= contextEnd
      else { throw ProviderError(Status.invalidArgument) }
      guard request.text_start <= globalStart, globalStart < request.text_end else { continue }

      let records = glyphRecordsByClusterStart[start16] ?? []
      let style = styleAt(globalOffset: globalStart, defaultStyle: defaultStyle, runs: styleRuns)
      let result = makeCluster(
        globalStart: globalStart,
        globalEnd: globalEnd,
        utf16Start: start16,
        utf16End: end16,
        line: line,
        records: records,
        style: style,
        bidiLevel: bidiLevels[start16],
        request: request
      )
      clusters.append(result)

      if records.contains(where: { $0.glyph == 0 }) {
        diagnostics.append(
          DiagnosticResult(
            start: globalStart,
            end: min(globalEnd, request.text_end),
            message: "Core Text could not resolve a glyph for this cluster"
          ))
      }
    }

    let ffiClusters = try clusters.map { result -> ViemShapedClusterV1 in
      var cluster = ViemShapedClusterV1()
      cluster.struct_size = UInt32(MemoryLayout<ViemShapedClusterV1>.size)
      cluster.reserved = 0
      cluster.text_start = result.start
      cluster.text_end = result.end
      cluster.advance = Float(result.advance)
      cluster.metrics = result.metrics.ffi
      cluster.typographic_bounds = result.typographicBounds.ffi
      cluster.ink_bounds = result.inkBounds.ffi
      cluster.bidi_level = UInt32(result.bidiLevel)
      cluster.fallback_font = arena.storeUTF8(result.fallbackFont)
      cluster.caret_stops = arena.storeCarets(result.carets)
      cluster.caret_stop_count = UInt64(result.carets.count)

      if request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
        request.has_render_run_policy == 1
      {
        guard
          let identifier = renderRegistry.install(
            result.renderResource,
            preferredIdentifier: result.renderIdentifier,
            generation: callbackGeneration,
            pin: true
          )
        else { throw ProviderError(Status.providerFailure) }
        arena.retainInstalledResource(identifier)
        cluster.has_render_run = 1
        cluster.render_run.owner = request.render_run_owner
        cluster.render_run.identifier = identifier
        cluster.render_run.metrics_generation = callbackGeneration
        cluster.render_run.threading = request.render_run_threading
        cluster.render_run.reserved = 0
      } else {
        cluster.has_render_run = 0
        cluster.render_run = ViemRenderRunHandleV1()
      }
      return cluster
    }

    guard
      request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_ONLY)
        || request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
      request.purpose != UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
        || (request.has_render_run_policy == 1
          && request.render_run_owner == renderRunOwner
          && request.render_run_threading == UInt32(VIEM_RENDER_THREADING_FRONTEND_MAIN))
    else { throw ProviderError(Status.invalidArgument) }

    let order = visualOrder(for: clusters.map(\.bidiLevel))
    let ffiDiagnostics = diagnostics.map { diagnostic -> ViemShapingDiagnosticV1 in
      var value = ViemShapingDiagnosticV1()
      value.struct_size = UInt32(MemoryLayout<ViemShapingDiagnosticV1>.size)
      value.reserved = 0
      value.text_start = diagnostic.start
      value.text_end = diagnostic.end
      value.message = arena.storeUTF8(diagnostic.message)
      return value
    }

    var response = ViemShapeResponseV1()
    response.struct_size = UInt32(MemoryLayout<ViemShapeResponseV1>.size)
    response.reserved = 0
    response.document_id = request.document_id
    response.document_revision = request.document_revision
    response.measurement_environment_id = measurementEnvironmentID
    response.metrics_generation = callbackGeneration
    response.text_start = request.text_start
    response.text_end = request.text_end
    response.clusters = arena.storeClusters(ffiClusters)
    response.cluster_count = UInt64(ffiClusters.count)
    response.visual_order = arena.storeVisualOrder(order.map(UInt64.init))
    response.visual_order_count = UInt64(order.count)
    response.default_metrics =
      metrics(for: defaultStyle.font, baselineShift: defaultStyle.baselineShift).ffi
    response.diagnostics = arena.storeDiagnostics(ffiDiagnostics)
    response.diagnostic_count = UInt64(ffiDiagnostics.count)
    return response
  }
}

private func viemCoreTextMetricsGeneration(_ context: UnsafeMutableRawPointer?) -> UInt64 {
  guard let context else { return 0 }
  return Unmanaged<CoreTextMeasurementProvider>.fromOpaque(context)
    .takeUnretainedValue().metricsGeneration
}

private func viemCoreTextShapeBatch(
  _ context: UnsafeMutableRawPointer?,
  _ requests: UnsafePointer<ViemShapeRequestV1>?,
  _ requestCount: UInt64,
  _ responses: UnsafeMutablePointer<ViemShapeResponseV1>?,
  _ responseCapacity: UInt64
) -> UInt32 {
  guard let context else { return Status.invalidArgument }
  return Unmanaged<CoreTextMeasurementProvider>.fromOpaque(context)
    .takeUnretainedValue()
    .shapeBatch(
      requests: requests,
      requestCount: requestCount,
      responses: responses,
      responseCapacity: responseCapacity
    )
}

private enum Status {
  static let ok: UInt32 = 0
  static let invalidArgument: UInt32 = 1
  static let invalidUTF8: UInt32 = 5
  static let providerFailure: UInt32 = 23
}

private struct ProviderError: Error {
  let status: UInt32

  init(_ status: UInt32) {
    self.status = status
  }
}

private func decode(_ slice: ViemUtf8Slice) throws -> String {
  guard slice.length <= UInt64(Int.max) else { throw ProviderError(Status.invalidArgument) }
  if slice.length == 0 { return "" }
  guard let data = slice.data else { throw ProviderError(Status.invalidArgument) }
  let bytes = UnsafeBufferPointer(start: data, count: Int(slice.length))
  guard let value = String(bytes: bytes, encoding: .utf8) else {
    throw ProviderError(Status.invalidUTF8)
  }
  return value
}

private func adding(_ lhs: UInt64, _ rhs: UInt64) -> UInt64? {
  let (result, overflow) = lhs.addingReportingOverflow(rhs)
  return overflow ? nil : result
}

private struct ResolvedStyle {
  let fontFamilies: [String]
  let font: CTFont
  let size: CGFloat
  let weight: CGFloat
  let slant: UInt32
  let letterSpacing: CGFloat
  let baselineShift: CGFloat
  let language: String?
  let script: String?
  let direction: UInt32
  let features: [(String, UInt32)]
  let syntheticBold: Bool

  init(_ source: ViemResolvedTextStyleV1, scale: CGFloat) throws {
    guard source.struct_size >= UInt32(MemoryLayout<ViemResolvedTextStyleV1>.size),
      source.reserved <= 1,
      source.size.isFinite, source.size > 0,
      source.weight.isFinite,
      source.letter_spacing.isFinite,
      source.baseline_shift.isFinite,
      source.has_language <= 1,
      source.has_script <= 1,
      source.slant == UInt32(VIEM_FONT_SLANT_UPRIGHT)
        || source.slant == UInt32(VIEM_FONT_SLANT_ITALIC)
        || source.slant == UInt32(VIEM_FONT_SLANT_OBLIQUE),
      source.direction == UInt32(VIEM_TEXT_DIRECTION_AUTO)
        || source.direction == UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT)
        || source.direction == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT)
    else { throw ProviderError(Status.invalidArgument) }

    var families: [String] = []
    if source.font_family_count > 0 {
      guard source.font_family_count <= UInt64(Int.max), let base = source.font_families else {
        throw ProviderError(Status.invalidArgument)
      }
      families.reserveCapacity(Int(source.font_family_count))
      for index in 0..<Int(source.font_family_count) {
        let family = try decode(base[index])
        guard !family.isEmpty else { throw ProviderError(Status.invalidArgument) }
        families.append(family)
      }
    }
    if families.isEmpty { families = [CoreTextMeasurementProvider.defaultFontFamily] }

    var decodedFeatures: [(String, UInt32)] = []
    if source.feature_count > 0 {
      guard source.feature_count <= UInt64(Int.max), let base = source.features else {
        throw ProviderError(Status.invalidArgument)
      }
      decodedFeatures.reserveCapacity(Int(source.feature_count))
      for index in 0..<Int(source.feature_count) {
        let feature = base[index]
        let tag = String(
          bytes: [feature.tag.0, feature.tag.1, feature.tag.2, feature.tag.3], encoding: .ascii)
        guard let tag, tag.utf8.allSatisfy({ (0x20...0x7E).contains($0) }) else {
          throw ProviderError(Status.invalidArgument)
        }
        decodedFeatures.append((tag, feature.value))
      }
    }

    fontFamilies = families
    size = CGFloat(source.size) * scale
    weight = CGFloat(source.weight)
    slant = source.slant
    letterSpacing = CGFloat(source.letter_spacing) * scale
    baselineShift = CGFloat(source.baseline_shift) * scale
    language = source.has_language == 1 ? try decode(source.language) : nil
    script = source.has_script == 1 ? try decode(source.script) : nil
    direction = source.direction
    features = decodedFeatures
    font = resolveFont(
      families: families,
      size: size,
      cssWeight: weight,
      slant: slant,
      features: decodedFeatures,
      relativeBold: source.reserved & 1 != 0
    )
    let baseWeight = EVFontCatalog.face(named: families.first ?? "")?.weight
    let target =
      source.reserved & 1 != 0
      ? min(Int(baseWeight ?? UInt16(max(1, min(1000, source.weight - 300)))) + 300, 1000)
      : Int(source.weight)
    syntheticBold = target >= 500 && Int(EVFontCatalog.weight(of: font)) < target
  }

  var attributes: [NSAttributedString.Key: Any] {
    var result: [NSAttributedString.Key: Any] = [
      NSAttributedString.Key(kCTFontAttributeName as String): font,
      NSAttributedString.Key(kCTBaselineOffsetAttributeName as String): baselineShift,
    ]
    result.merge(letterSpacingAttributes(letterSpacing)) { _, value in value }
    if syntheticBold {
      result[NSAttributedString.Key(kCTStrokeWidthAttributeName as String)] = -3.0
    }
    if let language, !language.isEmpty {
      result[NSAttributedString.Key(kCTLanguageAttributeName as String)] = language
    }
    if direction == UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT)
      || direction == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT)
    {
      let writingDirection =
        direction == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT)
        ? NSWritingDirection.rightToLeft.rawValue
        : NSWritingDirection.leftToRight.rawValue
      result[NSAttributedString.Key(kCTWritingDirectionAttributeName as String)] = [
        NSNumber(value: writingDirection | NSWritingDirectionFormatType.override.rawValue)
      ]
    }
    return result
  }
}

private struct StyleRun {
  let start: UInt64
  let end: UInt64
  let style: ResolvedStyle
}

private func readStyleRuns(_ request: ViemShapeRequestV1, scale: CGFloat) throws -> [StyleRun] {
  guard request.style_run_count <= UInt64(Int.max) else {
    throw ProviderError(Status.invalidArgument)
  }
  if request.style_run_count == 0 { return [] }
  guard let base = request.style_runs else { throw ProviderError(Status.invalidArgument) }
  var result: [StyleRun] = []
  result.reserveCapacity(Int(request.style_run_count))
  var previousEnd: UInt64?
  for index in 0..<Int(request.style_run_count) {
    let run = base[index]
    guard run.struct_size >= UInt32(MemoryLayout<ViemShapeStyleRunV1>.size),
      run.reserved == 0,
      run.text_start <= run.text_end,
      previousEnd.map({ $0 <= run.text_start }) ?? true
    else { throw ProviderError(Status.invalidArgument) }
    result.append(
      StyleRun(
        start: run.text_start,
        end: run.text_end,
        style: try ResolvedStyle(run.style, scale: scale)
      ))
    previousEnd = run.text_end
  }
  return result
}

private func makeAttributedString(
  _ text: String,
  contextStart: UInt64,
  indexMap: TextIndexMap,
  defaultStyle: ResolvedStyle,
  styleRuns: [StyleRun],
  paragraphDirection: UInt32
) throws -> NSAttributedString {
  let attributed = NSMutableAttributedString(string: text)
  let fullRange = NSRange(location: 0, length: indexMap.utf16Length)
  attributed.setAttributes(defaultStyle.attributes, range: fullRange)

  guard let contextEnd = adding(contextStart, UInt64(text.utf8.count)) else {
    throw ProviderError(Status.invalidArgument)
  }
  for run in styleRuns {
    let globalStart = max(run.start, contextStart)
    let globalEnd = min(run.end, contextEnd)
    guard globalStart < globalEnd else { continue }
    let localStart = Int(globalStart - contextStart)
    let localEnd = Int(globalEnd - contextStart)
    guard let start16 = indexMap.utf16Offset(forUTF8: localStart),
      let end16 = indexMap.utf16Offset(forUTF8: localEnd)
    else { throw ProviderError(Status.invalidArgument) }
    attributed.setAttributes(
      run.style.attributes,
      range: NSRange(location: start16, length: end16 - start16)
    )
  }

  let paragraph = NSMutableParagraphStyle()
  switch paragraphDirection {
  case UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT):
    paragraph.baseWritingDirection = .leftToRight
  case UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT):
    paragraph.baseWritingDirection = .rightToLeft
  default:
    paragraph.baseWritingDirection = .natural
  }
  attributed.addAttribute(.paragraphStyle, value: paragraph, range: fullRange)
  return attributed
}

/// Tracking adds points after shaping while retaining the font's pair kerning.
/// `kCTKernAttributeName = 0` explicitly disables kerning and must not represent
/// the ordinary zero letter-spacing value.
public func letterSpacingAttributes(_ spacing: CGFloat) -> [NSAttributedString.Key: Any] {
  // Tracking otherwise suppresses standard ligatures, including an explicit
  // font `liga=1`. Restore the normal attribute policy; font `liga=0` still wins.
  spacing == 0 ? [:] : [.tracking: spacing, .ligature: 1]
}

public func resolveFont(
  families: [String],
  size: CGFloat,
  cssWeight: CGFloat,
  slant: UInt32,
  features: [(String, UInt32)],
  relativeBold: Bool = false
) -> CTFont {
  // This also covers previews and clipboard fonts that do not pass through
  // the portable layout resolver. Kerning always follows the font's default.
  let features = features.filter { $0.0 != "kern" }
  let requestedIndex = families.firstIndex { !EVFontCatalog.faces(for: $0).isEmpty }
  let requested = requestedIndex.map { families[$0] } ?? CoreTextMeasurementProvider.defaultFontFamily
  let base = EVFontCatalog.baseFont(named: requested, size: size)
  let faces = EVFontCatalog.faces(for: requested)
  let exactFace = faces.first { $0.postScriptName.caseInsensitiveCompare(requested) == .orderedSame }
  let targetWidth = exactFace?.width ?? 0
  let nearestWidth = faces.map { abs($0.width - targetWidth) }.min() ?? 0
  let matchingWidth = faces.filter { abs($0.width - targetWidth) <= nearestWidth + 0.0001 }
  let wantsItalic = slant != UInt32(VIEM_FONT_SLANT_UPRIGHT)
  let matchingSlant = matchingWidth.filter { $0.italic == wantsItalic }
  let available = matchingSlant.isEmpty ? matchingWidth : matchingSlant
  let targetWeight =
    relativeBold && exactFace != nil
    ? EVFontCatalog.boldWeight(baseWeight: exactFace!.weight, faces: available)
    : UInt16(max(1, min(1000, cssWeight.rounded())))
  let preferred: EVFontFace?
  if !relativeBold, let exactFace, exactFace.weight == targetWeight, exactFace.italic == wantsItalic {
    preferred = exactFace
  } else if relativeBold {
    preferred =
      available.filter { $0.weight >= targetWeight }.min { $0.weight < $1.weight }
      ?? available.max { $0.weight < $1.weight }
  } else {
    preferred = available.min {
      abs(Int($0.weight) - Int(targetWeight)) < abs(Int($1.weight) - Int(targetWeight))
    }
  }
  var member = preferred.flatMap { EVFontCatalog.font(for: $0, size: size) } ?? base
  var symbolic: CTFontSymbolicTraits = []
  if relativeBold || targetWeight >= 600 { symbolic.insert(.traitBold) }
  if wantsItalic { symbolic.insert(.traitItalic) }
  // Ask Core Text for a native member first. If no heavier/italic member
  // exists, descriptor traits request the platform's synthetic treatment.
  let needsSyntheticBold = relativeBold && (preferred?.weight ?? 0) < targetWeight
  let needsSyntheticItalic = wantsItalic && !(preferred?.italic ?? false)
  var traits: [CFString: Any] = [:]
  if needsSyntheticBold || needsSyntheticItalic {
    member = CTFontCreateCopyWithSymbolicTraits(member, size, nil, symbolic, symbolic) ?? member
    traits[kCTFontSymbolicTrait] = symbolic.rawValue
    if needsSyntheticBold { traits[kCTFontWeightTrait] = 0.4 }
    if needsSyntheticItalic { traits[kCTFontSlantTrait] = 0.2 }
  }

  var descriptorAttributes: [CFString: Any] = [:]
  if !traits.isEmpty {
    let combined = NSMutableDictionary(dictionary: CTFontCopyTraits(member) as NSDictionary)
    for (key, value) in traits { combined[key] = value }
    descriptorAttributes[kCTFontTraitsAttribute] = combined
  }
  let cascade = families.dropFirst(requestedIndex.map { $0 + 1 } ?? families.count).compactMap { family -> CTFontDescriptor? in
    EVFontCatalog.availableFont(named: family, size: size).map(CTFontCopyFontDescriptor)
  }
  if !cascade.isEmpty {
    descriptorAttributes[kCTFontCascadeListAttribute] = cascade
  }
  if !features.isEmpty {
    descriptorAttributes[kCTFontFeatureSettingsAttribute] = features.map {
      [
        kCTFontOpenTypeFeatureTag: $0.0,
        kCTFontOpenTypeFeatureValue: NSNumber(value: $0.1),
      ] as [CFString: Any]
    }
  }

  let descriptor = CTFontDescriptorCreateCopyWithAttributes(
    CTFontCopyFontDescriptor(member),
    descriptorAttributes as CFDictionary
  )
  let resolved = CTFontCreateWithFontDescriptor(descriptor, size, nil)
  if wantsItalic && !CTFontGetSymbolicTraits(resolved).contains(.traitItalic) {
    var matrix = CTFontGetMatrix(resolved)
    matrix.c += 0.2
    return CTFontCreateCopyWithAttributes(resolved, size, &matrix, nil)
  }
  return resolved
}

private final class TextIndexMap {
  struct Boundary {
    let utf8: Int
    let utf16: Int
  }

  let boundaries: [Boundary]
  let utf16Length: Int
  private let utf8ToUTF16: [Int: Int]
  private let utf16ToUTF8: [Int: Int]

  init(_ text: String) {
    var values: [Boundary] = []
    var utf8Offset = 0
    var utf16Offset = 0
    for character in text {
      values.append(Boundary(utf8: utf8Offset, utf16: utf16Offset))
      utf8Offset += character.utf8.count
      utf16Offset += character.utf16.count
    }
    values.append(Boundary(utf8: utf8Offset, utf16: utf16Offset))
    boundaries = values
    utf16Length = utf16Offset
    utf8ToUTF16 = Dictionary(uniqueKeysWithValues: values.map { ($0.utf8, $0.utf16) })
    utf16ToUTF8 = Dictionary(uniqueKeysWithValues: values.map { ($0.utf16, $0.utf8) })
  }

  func utf16Offset(forUTF8 offset: Int) -> Int? {
    utf8ToUTF16[offset]
  }

  func utf8Offset(forUTF16 offset: Int) -> Int? {
    utf16ToUTF8[offset]
  }

  func graphemeStart(containingUTF16 offset: Int) -> Int {
    var lower = 0
    var upper = boundaries.count
    while lower < upper {
      let middle = lower + (upper - lower) / 2
      if boundaries[middle].utf16 <= offset {
        lower = middle + 1
      } else {
        upper = middle
      }
    }
    return boundaries[max(lower - 1, 0)].utf16
  }
}

private struct GlyphRecord {
  let run: Int
  let font: CTFont
  let glyph: CGGlyph
  let position: CGPoint
  let advance: CGSize
  let stringIndex: Int
}

private func extractGlyphRecords(_ line: CTLine) -> [GlyphRecord] {
  let runs = CTLineGetGlyphRuns(line) as! [CTRun]
  var result: [GlyphRecord] = []
  for runIndex in 0..<runs.count {
    let run = runs[runIndex]
    let count = CTRunGetGlyphCount(run)
    guard count > 0 else { continue }
    var glyphs = Array(repeating: CGGlyph(), count: count)
    var positions = Array(repeating: CGPoint.zero, count: count)
    var advances = Array(repeating: CGSize.zero, count: count)
    var indices = Array(repeating: CFIndex(), count: count)
    glyphs.withUnsafeMutableBufferPointer { CTRunGetGlyphs(run, CFRange(), $0.baseAddress!) }
    positions.withUnsafeMutableBufferPointer { CTRunGetPositions(run, CFRange(), $0.baseAddress!) }
    advances.withUnsafeMutableBufferPointer { CTRunGetAdvances(run, CFRange(), $0.baseAddress!) }
    indices.withUnsafeMutableBufferPointer {
      CTRunGetStringIndices(run, CFRange(), $0.baseAddress!)
    }

    let attributes = CTRunGetAttributes(run) as NSDictionary
    guard let fontValue = attributes[kCTFontAttributeName] else { continue }
    let font = fontValue as! CTFont
    let runRange = CTRunGetStringRange(run)
    for glyphIndex in 0..<count {
      let rawIndex = indices[glyphIndex]
      let stringIndex = rawIndex == kCFNotFound ? runRange.location : rawIndex
      result.append(
        GlyphRecord(
          run: runIndex,
          font: font,
          glyph: glyphs[glyphIndex],
          position: positions[glyphIndex],
          advance: advances[glyphIndex],
          stringIndex: max(stringIndex, 0)
        ))
    }
  }
  return result
}

private func clusterBoundaries(for records: [GlyphRecord], map: TextIndexMap) -> [Int] {
  var values = Set([0, map.utf16Length])
  for record in records {
    values.insert(map.graphemeStart(containingUTF16: record.stringIndex))
  }
  return values.sorted()
}

private struct Metrics {
  let ascent: CGFloat
  let descent: CGFloat
  let leading: CGFloat

  var ffi: ViemTextMetricsV1 {
    var result = ViemTextMetricsV1()
    result.ascent = Float(ascent)
    result.descent = Float(descent)
    result.leading = Float(leading)
    return result
  }
}

private func metrics(for font: CTFont, baselineShift: CGFloat) -> Metrics {
  Metrics(
    ascent: CTFontGetAscent(font) + max(baselineShift, 0),
    descent: CTFontGetDescent(font) + max(-baselineShift, 0),
    leading: max(CTFontGetLeading(font), 0)
  )
}

private struct Bounds {
  let x: CGFloat
  let y: CGFloat
  let width: CGFloat
  let height: CGFloat

  var ffi: ViemShapedBoundsV1 {
    var result = ViemShapedBoundsV1()
    result.x = Float(x)
    result.y = Float(y)
    result.width = Float(max(width, 0))
    result.height = Float(max(height, 0))
    return result
  }
}

private struct ClusterResult {
  let start: UInt64
  let end: UInt64
  let advance: CGFloat
  let metrics: Metrics
  let typographicBounds: Bounds
  let inkBounds: Bounds
  let bidiLevel: UInt8
  let fallbackFont: String
  let carets: [ViemClusterCaretStopV1]
  let renderResource: CoreTextRenderRegistry.Resource
  let renderIdentifier: UInt64
}

private struct DiagnosticResult {
  let start: UInt64
  let end: UInt64
  let message: String
}

private func makeCluster(
  globalStart: UInt64,
  globalEnd: UInt64,
  utf16Start: Int,
  utf16End: Int,
  line: CTLine,
  records: [GlyphRecord],
  style: ResolvedStyle,
  bidiLevel: UInt8,
  request: ViemShapeRequestV1
) -> ClusterResult {
  var secondaryStart: CGFloat = 0
  var secondaryEnd: CGFloat = 0
  let startOffset = CTLineGetOffsetForStringIndex(line, utf16Start, &secondaryStart)
  let endOffset = CTLineGetOffsetForStringIndex(line, utf16End, &secondaryEnd)
  // A bidi boundary has two caret positions. Core Text's primary positions
  // at the two string endpoints may belong to opposite directional runs;
  // subtracting them can measure an entire run instead of this cluster.
  // Signed glyph advances identify the matching pair without measuring
  // across the opposite bidi caret. Preserve negative positioning advances.
  let offsets = [startOffset, secondaryStart].flatMap { left in
    [endOffset, secondaryEnd].map { right in (left, right) }
  }
  let estimate = abs(records.reduce(0) { $0 + $1.advance.width })
  let endpoints = offsets.min {
    abs(abs($0.1 - $0.0) - estimate) < abs(abs($1.1 - $1.0) - estimate)
  }!
  var advance = abs(endpoints.1 - endpoints.0)
  if records.isEmpty { advance = abs(endOffset - startOffset) }
  advance = max(advance, 0)
  let visualLeft = min(endpoints.0, endpoints.1)

  let fonts = records.map(\.font)
  let allFonts = fonts.isEmpty ? [style.font] : fonts
  var clusterMetrics = Metrics(ascent: 0, descent: 0, leading: 0)
  for font in allFonts {
    let candidate = metrics(for: font, baselineShift: style.baselineShift)
    clusterMetrics = Metrics(
      ascent: max(clusterMetrics.ascent, candidate.ascent),
      descent: max(clusterMetrics.descent, candidate.descent),
      leading: max(clusterMetrics.leading, candidate.leading)
    )
  }

  var inkRect = CGRect.null
  for record in records {
    var glyph = record.glyph
    var bounds = CGRect.zero
    _ = CTFontGetBoundingRectsForGlyphs(record.font, .default, &glyph, &bounds, 1)
    let rect = CGRect(
      x: record.position.x + bounds.minX - visualLeft,
      y: -(record.position.y + bounds.maxY),
      width: max(bounds.width, 0),
      height: max(bounds.height, 0)
    )
    inkRect = inkRect.union(rect)
  }
  if inkRect.isNull || !inkRect.origin.x.isFinite || !inkRect.origin.y.isFinite {
    inkRect = CGRect(
      x: 0, y: -clusterMetrics.ascent, width: advance,
      height: clusterMetrics.ascent + clusterMetrics.descent)
  }

  if style.syntheticBold {
    inkRect = inkRect.insetBy(dx: -style.size * 0.015, dy: -style.size * 0.015)
  }
  let rtl = bidiLevel % 2 == 1
  let startInline = rtl ? advance : 0
  let endInline = rtl ? 0 : advance
  var startCaret = ViemClusterCaretStopV1()
  startCaret.text_offset = globalStart
  startCaret.inline_offset = Float(startInline)
  startCaret.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
  var endCaret = ViemClusterCaretStopV1()
  endCaret.text_offset = globalEnd
  endCaret.inline_offset = Float(endInline)
  endCaret.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)

  let grouped = Dictionary(grouping: records, by: \.run).keys.sorted().map {
    run -> CoreTextRenderRegistry.GlyphBatch in
    let members = records.filter { $0.run == run }
    return CoreTextRenderRegistry.GlyphBatch(
      font: members[0].font,
      strokeWidth: style.syntheticBold ? style.size * 0.03 : 0,
      glyphs: members.map(\.glyph),
      positions: members.map { CGPoint(x: $0.position.x - visualLeft, y: $0.position.y) }
    )
  }
  var signature = StableBytes()
  signature.append(globalEnd - globalStart)
  signature.append(request.metrics_generation)
  signature.append(Float(advance).bitPattern)
  signature.append(UInt64(bidiLevel))
  // Identical whitespace glyphs can carry distinct marker inheritance even
  // when their shaping geometry is equal (for example, language alone).
  signature.append(Float(style.baselineShift).bitPattern)
  signature.append(Float(style.letterSpacing).bitPattern)
  signature.append(style.direction)
  signature.append(UInt8(style.language == nil ? 0 : 1))
  if let language = style.language { signature.append(language) }
  for batch in grouped {
    signature.append(CTFontCopyPostScriptName(batch.font) as String)
    signature.append(Float(batch.strokeWidth).bitPattern)
    signature.append(Float(CTFontGetMatrix(batch.font).c).bitPattern)
    for (glyph, position) in zip(batch.glyphs, batch.positions) {
      signature.append(UInt64(glyph))
      signature.append(Float(position.x).bitPattern)
      signature.append(Float(position.y).bitPattern)
    }
  }
  let bytes = signature.bytes
  let resource = CoreTextRenderRegistry.Resource(
    signature: bytes,
    batches: grouped,
    isColorGlyph: allFonts.contains { CTFontGetSymbolicTraits($0).rawValue & (1 << 13) != 0 },
    textAttributes: .init(baselineShift: style.baselineShift,
      letterSpacing: style.letterSpacing, language: style.language,
      writingDirection: style.direction == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT) ? .rightToLeft
        : style.direction == UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT) ? .leftToRight : .natural)
  )
  let fallback = allFonts.map { CTFontCopyPostScriptName($0) as String }
    .reduce(into: [String]()) { names, name in
      if !names.contains(name) { names.append(name) }
    }
    .joined(separator: ", ")

  return ClusterResult(
    start: globalStart,
    end: globalEnd,
    advance: advance,
    metrics: clusterMetrics,
    typographicBounds: Bounds(
      x: 0,
      y: -clusterMetrics.ascent,
      width: advance,
      height: clusterMetrics.ascent + clusterMetrics.descent
    ),
    inkBounds: Bounds(
      x: inkRect.minX,
      y: inkRect.minY,
      width: inkRect.width,
      height: inkRect.height
    ),
    bidiLevel: bidiLevel,
    fallbackFont: fallback.isEmpty ? CoreTextMeasurementProvider.defaultFontFamily : fallback,
    carets: [startCaret, endCaret],
    renderResource: resource,
    renderIdentifier: stableHash(bytes)
  )
}

private func styleAt(globalOffset: UInt64, defaultStyle: ResolvedStyle, runs: [StyleRun])
  -> ResolvedStyle
{
  var lower = 0
  var upper = runs.count
  while lower < upper {
    let middle = lower + (upper - lower) / 2
    if runs[middle].start <= globalOffset {
      lower = middle + 1
    } else {
      upper = middle
    }
  }
  guard lower > 0 else { return defaultStyle }
  let candidate = runs[lower - 1]
  return globalOffset < candidate.end ? candidate.style : defaultStyle
}

private func resolvedBidiLevels(
  _ attributed: NSAttributedString,
  paragraphDirection: UInt32
) -> [UInt8] {
  guard attributed.length > 0 else { return [] }

  // CTRunStatus exposes only direction parity, which cannot distinguish a
  // nested level-2/3 embedding or isolate. Resolve the full UAX #9 levels from
  // the exact attributed input that Core Text shapes, including any explicit
  // writing-direction attributes supplied by style runs.
  let baseDirection: Int8
  switch paragraphDirection {
  case UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT):
    baseDirection = Int8(NSWritingDirection.leftToRight.rawValue)
  case UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT):
    baseDirection = Int8(NSWritingDirection.rightToLeft.rawValue)
  default:
    baseDirection = Int8(NSWritingDirection.natural.rawValue)
  }

  var levels = Array(repeating: UInt8(0), count: attributed.length)
  var resolvedDirections = Array(repeating: UInt8(0), count: attributed.length)
  levels.withUnsafeMutableBufferPointer { levelBuffer in
    resolvedDirections.withUnsafeMutableBufferPointer { directionBuffer in
      _ = CFAttributedStringGetBidiLevelsAndResolvedDirections(
        attributed,
        CFRange(location: 0, length: attributed.length),
        baseDirection,
        levelBuffer.baseAddress!,
        directionBuffer.baseAddress!
      )
    }
  }
  return levels
}

private func visualOrder(for levels: [UInt8]) -> [Int] {
  var order = Array(levels.indices)
  guard let maximum = levels.max(),
    let minimumOdd = levels.filter({ $0 % 2 == 1 }).min()
  else { return order }
  if minimumOdd > maximum { return order }
  for level in stride(from: maximum, through: minimumOdd, by: -1) {
    var start = 0
    while start < order.count {
      while start < order.count, levels[order[start]] < level { start += 1 }
      var end = start
      while end < order.count, levels[order[end]] >= level { end += 1 }
      order[start..<end].reverse()
      start = end
    }
  }
  return order
}

private struct StableBytes {
  private(set) var bytes: [UInt8] = []

  mutating func append<T: FixedWidthInteger>(_ value: T) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
  }

  mutating func append(_ value: String) {
    bytes.append(contentsOf: value.utf8)
    bytes.append(0)
  }
}

private func stableHash(_ bytes: [UInt8]) -> UInt64 {
  var result: UInt64 = 0xcbf2_9ce4_8422_2325
  for byte in bytes {
    result ^= UInt64(byte)
    result = result &* 0x0000_0100_0000_01b3
  }
  return result == 0 ? 1 : result
}

private final class ResponseArena {
  private let registry: CoreTextRenderRegistry
  private let generation: UInt64
  private var installedIdentifiers: [UInt64] = []
  init(registry: CoreTextRenderRegistry, generation: UInt64) {
    self.registry = registry; self.generation = generation
  }
  func retainInstalledResource(_ identifier: UInt64) { installedIdentifiers.append(identifier) }
  private var bytes: [(UnsafeMutablePointer<UInt8>, Int)] = []
  private var carets: [(UnsafeMutablePointer<ViemClusterCaretStopV1>, Int)] = []
  private var clusters: [(UnsafeMutablePointer<ViemShapedClusterV1>, Int)] = []
  private var visualOrders: [(UnsafeMutablePointer<UInt64>, Int)] = []
  private var diagnostics: [(UnsafeMutablePointer<ViemShapingDiagnosticV1>, Int)] = []

  deinit {
    registry.releaseResponseResources(identifiers: installedIdentifiers, generation: generation)
    for (pointer, count) in bytes {
      pointer.deinitialize(count: count)
      pointer.deallocate()
    }
    for (pointer, count) in carets {
      pointer.deinitialize(count: count)
      pointer.deallocate()
    }
    for (pointer, count) in clusters {
      pointer.deinitialize(count: count)
      pointer.deallocate()
    }
    for (pointer, count) in visualOrders {
      pointer.deinitialize(count: count)
      pointer.deallocate()
    }
    for (pointer, count) in diagnostics {
      pointer.deinitialize(count: count)
      pointer.deallocate()
    }
  }

  func storeUTF8(_ value: String) -> ViemUtf8Slice {
    let values = Array(value.utf8)
    guard !values.isEmpty else { return ViemUtf8Slice() }
    let pointer = UnsafeMutablePointer<UInt8>.allocate(capacity: values.count)
    pointer.initialize(from: values, count: values.count)
    bytes.append((pointer, values.count))
    var slice = ViemUtf8Slice()
    slice.data = UnsafePointer(pointer)
    slice.length = UInt64(values.count)
    return slice
  }

  func storeCarets(_ values: [ViemClusterCaretStopV1]) -> UnsafePointer<ViemClusterCaretStopV1>? {
    store(values, in: &carets)
  }

  func storeClusters(_ values: [ViemShapedClusterV1]) -> UnsafePointer<ViemShapedClusterV1>? {
    store(values, in: &clusters)
  }

  func storeVisualOrder(_ values: [UInt64]) -> UnsafePointer<UInt64>? {
    store(values, in: &visualOrders)
  }

  func storeDiagnostics(_ values: [ViemShapingDiagnosticV1]) -> UnsafePointer<
    ViemShapingDiagnosticV1
  >? {
    store(values, in: &diagnostics)
  }

  private func store<T>(
    _ values: [T],
    in allocations: inout [(UnsafeMutablePointer<T>, Int)]
  ) -> UnsafePointer<T>? {
    guard !values.isEmpty else { return nil }
    let pointer = UnsafeMutablePointer<T>.allocate(capacity: values.count)
    pointer.initialize(from: values, count: values.count)
    allocations.append((pointer, values.count))
    return UnsafePointer(pointer)
  }
}

private func viemCoreTextRetainRenderRuns(
  _ context: UnsafeMutableRawPointer?, _ handles: UnsafePointer<ViemRenderRunHandleV1>?,
  _ count: UInt64
) -> UnsafeMutableRawPointer? {
  guard let context, let handles, count > 0, count <= UInt64(Int.max) else { return nil }
  let provider = Unmanaged<CoreTextMeasurementProvider>.fromOpaque(context).takeUnretainedValue()
  let values = UnsafeBufferPointer(start: handles, count: Int(count))
  let generation = values[0].metrics_generation
  guard values.allSatisfy({ $0.owner == provider.renderRunOwner && $0.metrics_generation == generation
    && $0.threading == UInt32(VIEM_RENDER_THREADING_FRONTEND_MAIN) && $0.reserved == 0 }),
    let lease = provider.renderRegistry.retain(identifiers: values.map(\.identifier), generation: generation)
  else { return nil }
  return Unmanaged.passRetained(lease).toOpaque()
}

private func viemCoreTextReleaseRenderRuns(_ context: UnsafeMutableRawPointer?) {
  guard let context else { return }
  Unmanaged<RenderResourceLease>.fromOpaque(context).release()
}
