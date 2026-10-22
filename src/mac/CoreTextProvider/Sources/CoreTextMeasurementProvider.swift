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
  private let imageResources: CoreTextImageResources
  private let documentImages: CoreTextDocumentImages
  let imageConsumerID = UUID()
  private let observesImages: Bool
  public var imagesDidChange: (@Sendable (UInt64, [String]) -> Void)?
  /// Bitmaps or placeholder states changed without changing layout geometry.
  /// Drawing reads them from the shared resources; only a repaint is needed.
  public var imageRastersDidChange: (@Sendable () -> Void)?
  /// Worker providers for speculative layout read unknown image dimensions
  /// from file headers, so the geometry they produce is final.
  private let probesImageDimensions: Bool
  /// Jobs remain pending until their main-thread resource publication completes.
  var pendingImageLoadCount: Int { imageResources.pendingLoadCount }
  var localImageLoadCountForTesting: Int { imageResources.localLoadCount }
  var imageDimensionProbeCountForTesting: Int { imageResources.dimensionProbeCount }
  var imageScaledRenderCountForTesting: Int { imageResources.scaledRenderCount }

  /// The frontend refreshes metrics after installing its current immutable frame
  /// when new visible locations need admission to the bounded preview queue.
  @discardableResult
  public func updateVisibleImages(_ renderRuns: [(identifier: UInt64, metricsGeneration: UInt64)],
    viewportID: UInt64) -> Bool {
    imageResources.setVisibleLocations(renderRegistry.inlineImageLocations(in: renderRuns), viewportID: viewportID, consumer: imageConsumerID)
  }

  public func canReloadImage(_ destination: String) -> Bool { imageResources.canReload(destination) }

  @discardableResult
  public func reloadImage(_ destination: String) -> Bool { imageResources.reload(destination) }

  public func requestImageDimensions(_ destination: String,
    completion: @escaping @MainActor @Sendable (CGSize?) -> Void) -> Operation {
    imageResources.requestDimensions(destination, consumer: imageConsumerID, completion: completion)
  }

  /// Bitmap and admission changes preserve measured document heights. The
  /// frontend acknowledges this resource-only generation before refreshing.
  public func invalidateImageResources(_ changedDimensions: [String] = []) {
    let change = advanceMetricsGeneration()
    imagesDidChange?(change.previous, changedDimensions)
  }

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
  private let shapingDidBegin: (@Sendable () throws -> Void)?

  public convenience init(
    measurementEnvironmentID: UInt64 = UInt64.random(in: 1...UInt64.max),
    initialMetricsGeneration: UInt64 = 1,
    documentImages: CoreTextDocumentImages = CoreTextDocumentImages()
  ) {
    self.init(
      measurementEnvironmentID: measurementEnvironmentID,
      initialMetricsGeneration: initialMetricsGeneration,
      shapingDidBegin: nil,
      documentImages: documentImages
    )
  }

  init(
    measurementEnvironmentID: UInt64,
    initialMetricsGeneration: UInt64,
    shapingDidBegin: (@Sendable () throws -> Void)?,
    sharedRegistry: CoreTextRenderRegistry? = nil,
    documentImages: CoreTextDocumentImages = CoreTextDocumentImages(),
    observeFontChanges: Bool = true,
    probesImageDimensions: Bool = false
  ) {
    self.probesImageDimensions = probesImageDimensions
    self.measurementEnvironmentID = measurementEnvironmentID
    let proposedOwner = measurementEnvironmentID ^ 0x4354_5255_4E53_4554
    renderRunOwner = proposedOwner == 0 ? 0x5649_454D : proposedOwner
    generation = max(initialMetricsGeneration, 1)
    renderRegistry = sharedRegistry ?? CoreTextRenderRegistry(generation: max(initialMetricsGeneration, 1))
    self.shapingDidBegin = shapingDidBegin
    self.documentImages = documentImages
    imageResources = documentImages.resources
    observesImages = sharedRegistry == nil
    if observesImages {
      imageResources.observe(imageConsumerID, resources: { [weak self] destinations in
        self?.invalidateImageResources(destinations)
      }, rasters: { [weak self] in self?.imageRastersDidChange?() })
      let resources = imageResources
      renderRegistry.inlineImagePresentation = { [weak resources] captured in
        resources?.presentation(matching: captured)
      }
      renderRegistry.inlineImageDisplayRaster = { [weak resources] image, pixels in
        resources?.displayRaster(for: image, pixels: pixels)
      }
    }

    guard observeFontChanges else { return }
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
    if observesImages { imageResources.removeConsumer(imageConsumerID) }
    if let localFontObserver {
      NotificationCenter.default.removeObserver(localFontObserver)
    }
    if let distributedFontObserver {
      DistributedNotificationCenter.default().removeObserver(distributedFontObserver)
    }
  }

  /// Independent callback storage for one immutable worker request. Metrics are
  /// frozen to the captured generation; stale installation is rejected by core.
  /// Glyph leases share the UI registry so completed rows remain drawable.
  public func makeWorkerProvider(probesImageDimensions: Bool = true) -> CoreTextMeasurementProvider {
    CoreTextMeasurementProvider(
      measurementEnvironmentID: measurementEnvironmentID,
      initialMetricsGeneration: metricsGeneration,
      shapingDidBegin: nil,
      sharedRegistry: renderRegistry,
      documentImages: documentImages,
      observeFontChanges: false,
      probesImageDimensions: probesImageDimensions)
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
    return advanceMetricsGeneration().current
  }

  private func advanceMetricsGeneration() -> (previous: UInt64, current: UInt64) {
    stateLock.lock()
    let previous = generation
    generation = generation &+ 1
    if generation == 0 { generation = 1 }
    let current = generation
    renderRegistry.retireAll(forNewGeneration: current)
    stateLock.unlock()
    return (previous, current)
  }

  /// Ends all response and render-resource lifetimes after a view has been
  /// removed from core. The provider object itself must still outlive the C
  /// provider table installed for that view.
  public func retireResources() {
    imagesDidChange = nil; imageRastersDidChange = nil
    if observesImages { imageResources.removeConsumer(imageConsumerID) }
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
    table.abi_version = UInt32(VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION)
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
    do { try shapingDidBegin?() } catch { return Status.providerFailure }

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
    guard request.inline_image_count <= 4096,
      request.inline_image_count == 0 || request.inline_images != nil else { throw ProviderError(Status.invalidArgument) }
    var images: [UInt64: (UInt64, String)] = [:]
    let imageTextBytes = request.inline_image_count == 0 ? [] : Array(fullText.utf8)
    for index in 0..<Int(request.inline_image_count) {
      let image = request.inline_images![index]
      guard image.text_start >= contextStart, image.text_end <= contextEnd,
            image.text_start < image.text_end, image.text_end - image.text_start == 3,
            imageTextBytes[Int(image.text_start - contextStart)..<Int(image.text_end - contextStart)].elementsEqual([0xEF, 0xBF, 0xBC])
      else { throw ProviderError(Status.invalidArgument) }
      images[image.text_start] = (image.text_end, try decode(image.destination))
    }
    let objectBoundaries = Set(images.flatMap { [Int($0.key - contextStart), Int($0.value.0 - contextStart)] })
    let indexMap = TextIndexMap(fullText, forcedBoundaries: objectBoundaries)
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
    for (start, (end, destination)) in images {
      guard start >= contextStart, end <= contextEnd,
            let start16 = indexMap.utf16Offset(forUTF8: Int(start - contextStart)),
            let end16 = indexMap.utf16Offset(forUTF8: Int(end - contextStart)) else { throw ProviderError(Status.invalidArgument) }
      let image = imageResources.image(for: destination, probe: probesImageDimensions)
      let style = styleAt(globalOffset: start, defaultStyle: defaultStyle, runs: styleRuns)
      let size = imageLayoutSize(image, style: style, scale: CGFloat(request.scale))
      let box = InlineImageMetrics(width: size.width, height: size.height)
      var callbacks = CTRunDelegateCallbacks(version: kCTRunDelegateVersion1,
        dealloc: { pointer in Unmanaged<InlineImageMetrics>.fromOpaque(pointer).release() },
        getAscent: { pointer in Unmanaged<InlineImageMetrics>.fromOpaque(pointer).takeUnretainedValue().height },
        getDescent: { _ in 0 },
        getWidth: { pointer in Unmanaged<InlineImageMetrics>.fromOpaque(pointer).takeUnretainedValue().width })
      let retained = Unmanaged.passRetained(box).toOpaque()
      guard let delegate = CTRunDelegateCreate(&callbacks, retained) else {
        Unmanaged<InlineImageMetrics>.fromOpaque(retained).release(); throw ProviderError(Status.providerFailure)
      }
      attributed.addAttribute(NSAttributedString.Key(kCTRunDelegateAttributeName as String), value: delegate,
        range: NSRange(location: start16, length: end16 - start16))
    }
    let bidiLevels = resolvedBidiLevels(
      attributed,
      paragraphDirection: request.paragraph_base_direction
    )
    let line = CTLineCreateWithAttributedString(attributed)
    let caretOffsets = CoreTextLineCaretOffsets(line)
    var fonts = ShapedFonts()
    let glyphRecords = extractGlyphRecords(line, fonts: &fonts)
    let clusterStarts = glyphRecords.map { indexMap.graphemeStart(containingUTF16: $0.stringIndex) }
    // Each cluster's records, in shaped order. Starts are usually already
    // ordered; right-to-left runs need the stable sort.
    var recordOrder = Array(glyphRecords.indices)
    if zip(clusterStarts, clusterStarts.dropFirst()).contains(where: { $0 > $1 }) {
      recordOrder.sort { clusterStarts[$0] != clusterStarts[$1] ? clusterStarts[$0] < clusterStarts[$1] : $0 < $1 }
    }
    var boundaryValues = clusterStarts
    boundaryValues.append(0)
    boundaryValues.append(indexMap.utf16Length)
    boundaryValues.append(contentsOf: objectBoundaries.compactMap { indexMap.utf16Offset(forUTF8: $0) })
    boundaryValues.sort()
    var clusterBoundaries: [Int] = []
    clusterBoundaries.reserveCapacity(boundaryValues.count)
    for value in boundaryValues where clusterBoundaries.last != value { clusterBoundaries.append(value) }

    var clusters: [ClusterResult] = []
    clusters.reserveCapacity(clusterBoundaries.count)
    var diagnostics: [DiagnosticResult] = []
    var cursor = 0
    // Records of the returned clusters, in cluster order, and their render
    // signatures in one buffer. Render resources are built only for new ones.
    var clusterRecords: [GlyphRecord] = []
    clusterRecords.reserveCapacity(glyphRecords.count)
    var signatures = RenderSignatures(capacity: glyphRecords.count * 24 + clusterBoundaries.count * 64)
    var clusterFonts: [ShapedFont] = []
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

      while cursor < recordOrder.count, clusterStarts[recordOrder[cursor]] < start16 { cursor += 1 }
      let recordStart = clusterRecords.count
      clusterFonts.removeAll(keepingCapacity: true)
      while cursor < recordOrder.count, clusterStarts[recordOrder[cursor]] == start16 {
        let record = glyphRecords[recordOrder[cursor]]
        clusterRecords.append(record)
        let font = fonts.fonts[record.font]
        if !clusterFonts.contains(where: { $0 === font }) { clusterFonts.append(font) }
        cursor += 1
      }
      let style = styleAt(globalOffset: globalStart, defaultStyle: defaultStyle, runs: styleRuns)
      if clusterFonts.isEmpty { clusterFonts.append(fonts.font(for: style.font)) }
      let result = makeCluster(
        globalStart: globalStart,
        globalEnd: globalEnd,
        utf16Start: start16,
        utf16End: end16,
        caretOffsets: caretOffsets,
        records: clusterRecords,
        recordRange: recordStart..<clusterRecords.count,
        fonts: fonts.fonts,
        clusterFonts: clusterFonts,
        style: style,
        bidiLevel: bidiLevels[start16],
        request: request,
        signatures: &signatures
      )
      clusters.append(result)

      if images[globalStart] == nil && clusterRecords[recordStart...].contains(where: { $0.glyph == 0 }) {
        diagnostics.append(
          DiagnosticResult(
            start: globalStart,
            end: min(globalEnd, request.text_end),
            message: "Core Text could not resolve a glyph for this cluster"
          ))
      }
    }

    guard
      request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_ONLY)
        || request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA),
      request.purpose != UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
        || (request.has_render_run_policy == 1
          && request.render_run_owner == renderRunOwner
          && request.render_run_threading == UInt32(VIEM_RENDER_THREADING_FRONTEND_MAIN))
    else { throw ProviderError(Status.invalidArgument) }
    let installsRenderRuns = request.purpose == UInt32(VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
      && request.has_render_run_policy == 1

    var ffiClusters: [ViemShapedClusterV1] = []
    ffiClusters.reserveCapacity(clusters.count)
    var caretStops: [ViemClusterCaretStopV1] = []
    caretStops.reserveCapacity(clusters.count * 2)
    // Inline images carry their own resource; text clusters are looked up by
    // signature first.
    var inlineImageResources: [Int: (resource: CoreTextRenderRegistry.Resource, preferredIdentifier: UInt64)] = [:]
    for result in clusters {
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
      var startCaret = result.startCaret
      var endCaret = result.endCaret
      if let (end, destination) = images[result.start] {
        guard end == result.end else { throw ProviderError(Status.providerFailure) }
        let image = imageResources.image(for: destination, probe: probesImageDimensions)
        let style = styleAt(globalOffset: result.start, defaultStyle: defaultStyle, runs: styleRuns)
        let size = imageLayoutSize(image, style: style, scale: CGFloat(request.scale))
        let width = Float(size.width)
        let height = Float(size.height)
        cluster.advance = width
        cluster.metrics.ascent = height; cluster.metrics.descent = 0; cluster.metrics.leading = 0
        cluster.typographic_bounds = ViemShapedBoundsV1(x: 0, y: -height, width: width, height: height)
        cluster.ink_bounds = cluster.typographic_bounds
        startCaret = ViemClusterCaretStopV1(); startCaret.text_offset = result.start; startCaret.inline_offset = result.bidiLevel % 2 == 0 ? 0 : width; startCaret.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        endCaret = ViemClusterCaretStopV1(); endCaret.text_offset = result.end; endCaret.inline_offset = result.bidiLevel % 2 == 0 ? width : 0; endCaret.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
        if installsRenderRuns {
          let imageSignature = ["image", image.identity.uuidString, String(request.scale),
            CTFontCopyPostScriptName(style.font) as String, String(describing: style.size)].joined(separator: ":")
          let signature = Array(signatures.bytes[result.signature]) + Array(imageSignature.utf8)
          inlineImageResources[ffiClusters.count] = (CoreTextRenderRegistry.Resource(signature: signature, batches: [],
            isColorGlyph: false, textAttributes: result.style.renderAttributes, inlineImage: image.withoutRaster(),
            inlineImageFont: style.font), stableHash(signature))
        }
      }
      caretStops.append(startCaret)
      caretStops.append(endCaret)
      cluster.has_render_run = 0
      cluster.render_run = ViemRenderRunHandleV1()
      ffiClusters.append(cluster)
    }
    // One allocation holds every cluster's start and end caret stops.
    let caretBase = arena.storeCarets(caretStops)
    for index in ffiClusters.indices {
      ffiClusters[index].caret_stops = caretBase.map { $0 + 2 * index }
      ffiClusters[index].caret_stop_count = 2
    }
    if installsRenderRuns {
      let identifiers = try installRenderResources(clusters, imageResources: inlineImageResources,
        records: clusterRecords, fonts: fonts.fonts, signatures: signatures, generation: callbackGeneration)
      for (index, identifier) in identifiers.enumerated() {
        arena.retainInstalledResource(identifier)
        ffiClusters[index].has_render_run = 1
        ffiClusters[index].render_run.owner = request.render_run_owner
        ffiClusters[index].render_run.identifier = identifier
        ffiClusters[index].render_run.metrics_generation = callbackGeneration
        ffiClusters[index].render_run.threading = request.render_run_threading
        ffiClusters[index].render_run.reserved = 0
      }
    }

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
    response.default_metrics = fontMetrics(defaultStyle.font).ffi
    response.default_font_is_monospaced = CTFontGetSymbolicTraits(defaultStyle.font).contains(.traitMonoSpace) ? 1 : 0
    response.diagnostics = arena.storeDiagnostics(ffiDiagnostics)
    response.diagnostic_count = UInt64(ffiDiagnostics.count)
    return response
  }
}

extension CoreTextMeasurementProvider {
  /// Pins one render resource per cluster, in cluster order. Most clusters
  /// repeat glyphs already registered, so text clusters are matched by
  /// signature first; glyph arrays are built only for new signatures, once per
  /// distinct signature in the response.
  fileprivate func installRenderResources(
    _ clusters: [ClusterResult],
    imageResources: [Int: (resource: CoreTextRenderRegistry.Resource, preferredIdentifier: UInt64)],
    records: [GlyphRecord],
    fonts: [ShapedFont],
    signatures: RenderSignatures,
    generation: UInt64
  ) throws -> [UInt64] {
    var lookups: [(preferredIdentifier: UInt64, signature: Range<Int>)] = []
    var lookupClusters: [Int] = []
    lookups.reserveCapacity(clusters.count)
    lookupClusters.reserveCapacity(clusters.count)
    for (index, cluster) in clusters.enumerated() where imageResources[index] == nil {
      lookups.append((cluster.renderIdentifier, cluster.signature))
      lookupClusters.append(index)
    }
    guard let existing = renderRegistry.pinExisting(lookups, signatures: signatures.bytes, generation: generation)
    else { throw ProviderError(Status.providerFailure) }
    var identifiers = [UInt64](repeating: 0, count: clusters.count)
    var missing = imageResources
    var built: [UInt64: [(signature: Range<Int>, resource: CoreTextRenderRegistry.Resource)]] = [:]
    for (lookup, cluster) in zip(existing, lookupClusters) {
      if let identifier = lookup {
        identifiers[cluster] = identifier
        continue
      }
      let result = clusters[cluster]
      let candidates = built[result.renderIdentifier] ?? []
      if let match = candidates.first(where: { signatures.bytes[$0.signature] == signatures.bytes[result.signature] }) {
        missing[cluster] = (match.resource, result.renderIdentifier)
      } else {
        let resource = makeRenderResource(result, records: records, fonts: fonts, signatures: signatures)
        built[result.renderIdentifier, default: []].append((result.signature, resource))
        missing[cluster] = (resource, result.renderIdentifier)
      }
    }
    if !missing.isEmpty {
      let order = missing.keys.sorted()
      // A generation change between the lookup and installation retired
      // every pin, including the matches above.
      guard let installed = renderRegistry.install(order.map { missing[$0]! }, generation: generation, pin: true)
      else { throw ProviderError(Status.providerFailure) }
      for (cluster, identifier) in zip(order, installed) { identifiers[cluster] = identifier }
    }
    return identifiers
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

private func imageLayoutSize(_ image: CoreTextInlineImage, style: ResolvedStyle, scale: CGFloat) -> CGSize {
  let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
  if image.state == .ready { return size }
  return CGSize(width: size.width, height: max(size.height, CTFontGetAscent(style.font) + CTFontGetDescent(style.font) + 16 * scale))
}

/// A class: clusters look up their style by offset, and copying a struct of
/// arrays and strings per cluster retained each member.
private final class ResolvedStyle {
  let fontFamilies: [String]
  let font: CTFont
  let size: CGFloat
  let weight: CGFloat
  let slant: UInt32
  let letterSpacing: CGFloat
  let baselineOffset: CGFloat
  let language: String?
  let script: String?
  let direction: UInt32
  let features: [(String, UInt32)]
  let syntheticBold: Bool
  let renderAttributes: CoreTextRenderAttributes

  init(_ source: ViemResolvedTextStyleV1, scale: CGFloat) throws {
    guard source.struct_size >= UInt32(MemoryLayout<ViemResolvedTextStyleV1>.size),
      source.reserved <= 1,
      source.size.isFinite, source.size > 0,
      source.weight.isFinite,
      source.letter_spacing.isFinite, source.baseline_offset.isFinite,
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
    baselineOffset = CGFloat(source.baseline_offset) * scale
    language = source.has_language == 1 ? try decode(source.language) : nil
    script = source.has_script == 1 ? try decode(source.script) : nil
    direction = source.direction
    features = decodedFeatures
    let faceName = try decode(source.font_face)
    let axes = try decode(source.font_axes)
    let relativeBold = source.reserved & 1 != 0
    let key = ResolvedFontKey(
      families: families, size: size, weight: source.weight, slant: slant,
      features: decodedFeatures.map { ResolvedFontKey.Feature(tag: $0.0, value: $0.1) },
      relativeBold: relativeBold, axes: axes, faceName: faceName)
    let resolved = ResolvedFontCache.shared.value(for: key) {
      let font = resolveFont(
        families: families,
        size: key.size,
        cssWeight: CGFloat(source.weight),
        slant: source.slant,
        features: decodedFeatures,
        relativeBold: relativeBold,
        axes: EVFontVariations.decode(axes),
        faceName: faceName
      )
      let baseWeight = EVFontCatalog.face(in: families.first ?? "", named: faceName)?.weight
      let target =
        relativeBold
        ? min(Int(baseWeight ?? UInt16(max(1, min(1000, source.weight - 300)))) + 300, 1000)
        : Int(source.weight)
      return ResolvedFontCache.Value(font: font, syntheticBold:
        !EVFontVariations.info(font: font).axes.contains(where: { $0.tag == "wght" }) && target >= 500
          && Int(EVFontCatalog.weight(of: font)) < target)
    }
    font = resolved.font
    syntheticBold = resolved.syntheticBold
    renderAttributes = CoreTextRenderAttributes(letterSpacing: letterSpacing, language: language,
      writingDirection: direction == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT) ? .rightToLeft
        : direction == UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT) ? .leftToRight : .natural,
      baselineOffset: baselineOffset)
  }

  var attributes: [NSAttributedString.Key: Any] {
    var result: [NSAttributedString.Key: Any] = [
      NSAttributedString.Key(kCTFontAttributeName as String): font,
    ]
    result[.baselineOffset] = baselineOffset
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

/// Every font input of a resolved text style, at its scaled size.
private struct ResolvedFontKey: Hashable {
  struct Feature: Hashable {
    let tag: String
    let value: UInt32
  }
  let families: [String]
  let size: CGFloat
  let weight: Float
  let slant: UInt32
  let features: [Feature]
  let relativeBold: Bool
  let axes: String
  let faceName: String
}

/// Shaping requests repeat the same few document styles for every paragraph,
/// and constructing their Core Text fonts dominated each request. Resolved
/// fonts are immutable, so retain them for the current font catalogue
/// generation. The lock covers only dictionary access: resolution runs
/// unlocked, never under the catalogue lock, and a result resolved across an
/// invalidation is returned without being retained.
private final class ResolvedFontCache: @unchecked Sendable {
  struct Value {
    let font: CTFont
    let syntheticBold: Bool
  }

  static let shared = ResolvedFontCache()
  /// Documents use few styles; zoom and theme edits add sizes. Clearing on
  /// overflow keeps the bound without per-hit bookkeeping.
  private static let capacity = 512
  private let lock = NSLock()
  private var generation: UInt64 = 0
  private var values: [ResolvedFontKey: Value] = [:]

  func value(for key: ResolvedFontKey, resolve: () -> Value) -> Value {
    let current = EVFontCatalog.cacheGeneration
    lock.lock()
    if generation != current {
      values.removeAll()
      generation = current
    }
    let cached = values[key]
    lock.unlock()
    if let cached { return cached }
    let value = resolve()
    let after = EVFontCatalog.cacheGeneration
    lock.lock()
    if after == current, generation == current {
      if values.count >= Self.capacity { values.removeAll(keepingCapacity: true) }
      values[key] = value
    }
    lock.unlock()
    return value
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
) throws -> NSMutableAttributedString {
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
  relativeBold: Bool = false,
  axes: [String: Double] = [:], faceName: String = ""
) -> CTFont {
  // This also covers previews and clipboard fonts that do not pass through
  // the portable layout resolver. Kerning always follows the font's default.
  let features = features.filter { $0.0 != "kern" }
  let requestedIndex = families.firstIndex { !EVFontCatalog.faces(for: $0).isEmpty }
  let requested = requestedIndex.map { families[$0] } ?? CoreTextMeasurementProvider.defaultFontFamily
  let base = (requestedIndex == 0 ? EVFontCatalog.face(in: requested, named: faceName) : nil)
    .flatMap { EVFontCatalog.font(for: $0, size: size) } ?? EVFontCatalog.baseFont(named: requested, size: size)
  let faces = EVFontCatalog.faces(for: requested)
  let exactFace = requestedIndex == 0 ? EVFontCatalog.face(in: requested, named: faceName) : nil
  let targetWidth = exactFace?.width ?? 0
  let nearestWidth = faces.map { abs($0.width - targetWidth) }.min() ?? 0
  let matchingWidth = faces.filter { abs($0.width - targetWidth) <= nearestWidth + 0.0001 }
  let wantsItalic = slant != UInt32(VIEM_FONT_SLANT_UPRIGHT) || (exactFace?.italic ?? false)
  let matchingSlant = matchingWidth.filter { $0.italic == wantsItalic }
  let available = matchingSlant.isEmpty ? matchingWidth : matchingSlant
  let targetWeight =
    UInt16(max(1, min(1000, cssWeight.rounded())))
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
  // Keep variation outlines before considering any synthetic treatment.
  let baseInfo = EVFontVariations.info(font: base)
  let hasBaseItalicAxis = baseInfo.axes.contains { $0.tag == "ital" || $0.tag == "slnt" }
  let variationBase = wantsItalic && !(exactFace?.italic ?? false) && !hasBaseItalicAxis ? member : base
  let variationInfo = EVFontVariations.info(font: variationBase)
  var savedCoordinates: [String: Double] = [:]
  if exactFace != nil, let native = CTFontCopyVariation(variationBase) as? [NSNumber: NSNumber] {
    // A named variable face supplies its own base design (e.g. mono/casual).
    // Preserve that design when no explicit coordinates override it. Weight
    // still follows the separately resolved base-weight declaration.
    for axis in variationInfo.axes where axis.tag != "wght" {
      if let value = native[NSNumber(value: EVFontVariations.identifier(axis.tag))] {
        savedCoordinates[axis.tag] = value.doubleValue
      }
    }
  }
  if requestedIndex == 0 { savedCoordinates.merge(axes) { _, explicit in explicit } }
  let coordinates = EVFontVariations.effective(variationInfo, saved: savedCoordinates, weight: Double(cssWeight), bold: relativeBold, slant: slant)
  if !coordinates.isEmpty {
    let variation = Dictionary(uniqueKeysWithValues: coordinates.map { (NSNumber(value: EVFontVariations.identifier($0.key)), NSNumber(value: $0.value)) })
    let descriptor = CTFontDescriptorCreateCopyWithAttributes(CTFontCopyFontDescriptor(variationBase), [kCTFontVariationAttribute: variation] as CFDictionary)
    member = CTFontCreateWithFontDescriptor(descriptor, size, nil)
  }
  var symbolic: CTFontSymbolicTraits = []
  if relativeBold || targetWeight >= 600 { symbolic.insert(.traitBold) }
  if wantsItalic { symbolic.insert(.traitItalic) }
  // Ask Core Text for a native member first. If no heavier/italic member
  // exists, descriptor traits request the platform's synthetic treatment.
  let needsSyntheticBold = relativeBold && !variationInfo.axes.contains(where: { $0.tag == "wght" }) && (preferred?.weight ?? 0) < targetWeight
  let hasItalicAxis = variationInfo.axes.contains { $0.tag == "ital" || $0.tag == "slnt" }
  let needsSyntheticItalic = wantsItalic && !hasItalicAxis && !(preferred?.italic ?? false)
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
    EVFontCatalog.faceForFamilyChange(to: family, currentFace: nil)
      .flatMap { EVFontCatalog.font(for: $0, size: size) }.map(CTFontCopyFontDescriptor)
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
  if wantsItalic && !hasItalicAxis && !CTFontGetSymbolicTraits(resolved).contains(.traitItalic) {
    var matrix = CTFontGetMatrix(resolved)
    matrix.c += 0.2
    return CTFontCreateCopyWithAttributes(resolved, size, &matrix, nil)
  }
  return resolved
}

private final class InlineImageMetrics {
  let width: CGFloat
  let height: CGFloat
  init(width: CGFloat, height: CGFloat) { self.width = width; self.height = height }
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

  init(_ text: String, forcedBoundaries: Set<Int> = []) {
    var values: [Boundary] = []
    var utf8Offset = 0
    var utf16Offset = 0
    for character in text {
      values.append(Boundary(utf8: utf8Offset, utf16: utf16Offset))
      utf8Offset += character.utf8.count
      utf16Offset += character.utf16.count
    }
    values.append(Boundary(utf8: utf8Offset, utf16: utf16Offset))
    if !forcedBoundaries.isEmpty {
      var scalar8 = 0; var scalar16 = 0
      for scalar in text.unicodeScalars {
        if forcedBoundaries.contains(scalar8) { values.append(Boundary(utf8: scalar8, utf16: scalar16)) }
        scalar8 += scalar.utf8.count; scalar16 += scalar.utf16.count
      }
      values = Dictionary(values.map { ($0.utf8, $0) }, uniquingKeysWith: { a, _ in a }).values.sorted { $0.utf8 < $1.utf8 }
    }
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

/// Font facts shared by every glyph shaped with one Core Text font.
private final class ShapedFont {
  let font: CTFont
  let postScriptName: String
  let metrics: Metrics
  let obliqueness: CGFloat
  let isColorGlyph: Bool

  init(_ font: CTFont) {
    self.font = font
    postScriptName = CTFontCopyPostScriptName(font) as String
    metrics = fontMetrics(font)
    obliqueness = CTFontGetMatrix(font).c
    isColorGlyph = CTFontGetSymbolicTraits(font).rawValue & (1 << 13) != 0
  }
}

/// The distinct fonts of one shape request, each queried once.
private struct ShapedFonts {
  private(set) var fonts: [ShapedFont] = []
  private var indices: [ObjectIdentifier: Int] = [:]

  mutating func index(of font: CTFont) -> Int {
    let key = ObjectIdentifier(font)
    if let index = indices[key] { return index }
    fonts.append(ShapedFont(font))
    indices[key] = fonts.count - 1
    return fonts.count - 1
  }

  mutating func font(for font: CTFont) -> ShapedFont { fonts[index(of: font)] }
}

private struct GlyphRecord {
  let run: Int
  /// Index in the request's `ShapedFonts`.
  let font: Int
  let glyph: CGGlyph
  let position: CGPoint
  let advance: CGFloat
  let stringIndex: Int
  /// The glyph's bounding rectangle in its font.
  let bounds: CGRect
}

private func extractGlyphRecords(_ line: CTLine, fonts: inout ShapedFonts) -> [GlyphRecord] {
  let runs = CTLineGetGlyphRuns(line) as! [CTRun]
  var result: [GlyphRecord] = []
  result.reserveCapacity(CTLineGetGlyphCount(line))
  for runIndex in 0..<runs.count {
    let run = runs[runIndex]
    let count = CTRunGetGlyphCount(run)
    guard count > 0 else { continue }
    let attributes = CTRunGetAttributes(run) as NSDictionary
    guard let fontValue = attributes[kCTFontAttributeName] else { continue }
    let font = fontValue as! CTFont
    let fontIndex = fonts.index(of: font)
    var glyphs = Array(repeating: CGGlyph(), count: count)
    var positions = Array(repeating: CGPoint.zero, count: count)
    var advances = Array(repeating: CGSize.zero, count: count)
    var indices = Array(repeating: CFIndex(), count: count)
    var bounds = Array(repeating: CGRect.zero, count: count)
    glyphs.withUnsafeMutableBufferPointer { CTRunGetGlyphs(run, CFRange(), $0.baseAddress!) }
    positions.withUnsafeMutableBufferPointer { CTRunGetPositions(run, CFRange(), $0.baseAddress!) }
    advances.withUnsafeMutableBufferPointer { CTRunGetAdvances(run, CFRange(), $0.baseAddress!) }
    indices.withUnsafeMutableBufferPointer {
      CTRunGetStringIndices(run, CFRange(), $0.baseAddress!)
    }
    glyphs.withUnsafeBufferPointer { glyphBuffer in
      bounds.withUnsafeMutableBufferPointer {
        _ = CTFontGetBoundingRectsForGlyphs(font, .default, glyphBuffer.baseAddress!, $0.baseAddress!, count)
      }
    }

    let runRange = CTRunGetStringRange(run)
    for glyphIndex in 0..<count {
      let rawIndex = indices[glyphIndex]
      let stringIndex = rawIndex == kCFNotFound ? runRange.location : rawIndex
      result.append(
        GlyphRecord(
          run: runIndex,
          font: fontIndex,
          glyph: glyphs[glyphIndex],
          position: positions[glyphIndex],
          advance: advances[glyphIndex].width,
          stringIndex: max(stringIndex, 0),
          bounds: bounds[glyphIndex]
        ))
    }
  }
  return result
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

private func fontMetrics(_ font: CTFont) -> Metrics {
  Metrics(
    ascent: CTFontGetAscent(font),
    descent: CTFontGetDescent(font),
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
  let startCaret: ViemClusterCaretStopV1
  let endCaret: ViemClusterCaretStopV1
  /// Render data, materialized only for a resource not already registered.
  let style: ResolvedStyle
  let recordRange: Range<Int>
  let visualLeft: CGFloat
  let isColorGlyph: Bool
  let signature: Range<Int>
  let renderIdentifier: UInt64
}

/// One response's render signatures, contiguous.
private struct RenderSignatures {
  private(set) var bytes: [UInt8] = []

  init(capacity: Int) { bytes.reserveCapacity(capacity) }

  /// Appends exactly `count` bytes written by `body` and returns their range
  /// and `stableHash`.
  mutating func append(count: Int, _ body: (inout SignatureWriter) -> Void) -> (Range<Int>, UInt64) {
    let start = bytes.count
    bytes.append(contentsOf: repeatElement(0, count: count))
    return bytes.withUnsafeMutableBytes { raw in
      var writer = SignatureWriter(raw.baseAddress!, offset: start)
      body(&writer)
      precondition(writer.offset == start + count, "render signature size")
      var hash: UInt64 = 0xcbf2_9ce4_8422_2325
      for byte in UnsafeRawBufferPointer(rebasing: raw[start..<start + count]) {
        hash = (hash ^ UInt64(byte)) &* 0x0000_0100_0000_01b3
      }
      return (start..<start + count, hash == 0 ? 1 : hash)
    }
  }
}

/// Writes the stable little-endian signature format into reserved storage.
private struct SignatureWriter {
  private let base: UnsafeMutableRawPointer
  private(set) var offset: Int

  init(_ base: UnsafeMutableRawPointer, offset: Int) {
    self.base = base
    self.offset = offset
  }

  mutating func write<T: FixedWidthInteger>(_ value: T) {
    var little = value.littleEndian
    withUnsafeBytes(of: &little) { (base + offset).copyMemory(from: $0.baseAddress!, byteCount: $0.count) }
    offset += MemoryLayout<T>.size
  }

  static func size(of value: String) -> Int { value.utf8.count + 1 }

  mutating func write(_ value: String) {
    for byte in value.utf8 {
      base.storeBytes(of: byte, toByteOffset: offset, as: UInt8.self)
      offset += 1
    }
    base.storeBytes(of: 0, toByteOffset: offset, as: UInt8.self)
    offset += 1
  }
}

/// Visits the adjacent records of each Core Text run within one cluster.
private func forEachGlyphBatch(_ records: ArraySlice<GlyphRecord>, _ body: (ArraySlice<GlyphRecord>) -> Void) {
  var batchStart = records.startIndex
  while batchStart < records.endIndex {
    let run = records[batchStart].run
    var batchEnd = batchStart
    while batchEnd < records.endIndex, records[batchEnd].run == run { batchEnd += 1 }
    body(records[batchStart..<batchEnd])
    batchStart = batchEnd
  }
}

private func makeRenderResource(_ result: ClusterResult, records: [GlyphRecord], fonts: [ShapedFont],
  signatures: RenderSignatures) -> CoreTextRenderRegistry.Resource {
  let style = result.style
  let strokeWidth = style.syntheticBold ? style.size * 0.03 : 0
  var batches: [CoreTextRenderRegistry.GlyphBatch] = []
  forEachGlyphBatch(records[result.recordRange]) { batch in
    batches.append(CoreTextRenderRegistry.GlyphBatch(
      font: fonts[batch.first!.font].font,
      strokeWidth: strokeWidth,
      glyphs: batch.map(\.glyph),
      positions: batch.map { CGPoint(x: $0.position.x - result.visualLeft, y: $0.position.y) }))
  }
  return CoreTextRenderRegistry.Resource(
    signature: Array(signatures.bytes[result.signature]),
    batches: batches,
    isColorGlyph: result.isColorGlyph,
    textAttributes: style.renderAttributes)
}

private struct DiagnosticResult {
  let start: UInt64
  let end: UInt64
  let message: String
}

/// A single traversal supplies the ordinary caret edges of this shaped line.
/// Core Text adjusts split bidi carets independently of the enumerated edges
/// (including tracking), so resolve those boundaries through its exact query.
/// Line endpoints also use the query: the paragraph direction can introduce a
/// secondary terminal caret without a corresponding enumerated edge.
/// This storage lives only for the current shape request.
struct CoreTextLineCaretOffsets {
  struct Pair {
    let primary: CGFloat
    let secondary: CGFloat
  }

  private let line: CTLine
  private let range: CFRange
  /// Edges by UTF-16 boundary, relative to the line's string range.
  private let values: [Pair?]

  init(_ line: CTLine) {
    self.line = line
    let range = CTLineGetStringRange(line)
    self.range = range
    var edges = [Pair?](repeating: nil, count: max(range.length + 1, 0))
    CTLineEnumerateCaretOffsets(line) { offset, index, leadingEdge, _ in
      // A trailing edge names the final UTF-16 unit of its character.
      let boundary = (leadingEdge ? index : index + 1) - range.location
      guard edges.indices.contains(boundary) else { return }
      if let previous = edges[boundary] {
        if previous.primary != offset {
          edges[boundary] = Pair(primary: previous.primary, secondary: offset)
        }
      } else {
        edges[boundary] = Pair(primary: offset, secondary: offset)
      }
    }
    values = edges
  }

  func offsets(at index: Int) -> Pair {
    if index != range.location, index != range.location + range.length,
      values.indices.contains(index - range.location),
      let pair = values[index - range.location], pair.primary == pair.secondary { return pair }
    var secondary: CGFloat = 0
    let primary = CTLineGetOffsetForStringIndex(line, index, &secondary)
    return Pair(primary: primary, secondary: secondary)
  }
}

private func makeCluster(
  globalStart: UInt64,
  globalEnd: UInt64,
  utf16Start: Int,
  utf16End: Int,
  caretOffsets: CoreTextLineCaretOffsets,
  records allRecords: [GlyphRecord],
  recordRange: Range<Int>,
  fonts: [ShapedFont],
  clusterFonts: [ShapedFont],
  style: ResolvedStyle,
  bidiLevel: UInt8,
  request: ViemShapeRequestV1,
  signatures: inout RenderSignatures
) -> ClusterResult {
  let records = allRecords[recordRange]
  let start = caretOffsets.offsets(at: utf16Start)
  let end = caretOffsets.offsets(at: utf16End)
  let startOffset = start.primary
  let secondaryStart = start.secondary
  let endOffset = end.primary
  let secondaryEnd = end.secondary
  // A bidi boundary has two caret positions. Core Text's primary positions
  // at the two string endpoints may belong to opposite directional runs;
  // subtracting them can measure an entire run instead of this cluster.
  // Signed glyph advances identify the matching pair without measuring
  // across the opposite bidi caret. Preserve negative positioning advances.
  let estimate = abs(records.reduce(0) { $0 + $1.advance })
  var endpoints = (startOffset, endOffset)
  var closest = CGFloat.infinity
  for left in [startOffset, secondaryStart] {
    for right in [endOffset, secondaryEnd] {
      let distance = abs(abs(right - left) - estimate)
      if distance < closest {
        closest = distance
        endpoints = (left, right)
      }
    }
  }
  var advance = abs(endpoints.1 - endpoints.0)
  if records.isEmpty { advance = abs(endOffset - startOffset) }
  advance = max(advance, 0)
  let visualLeft = min(endpoints.0, endpoints.1)

  // The distinct fonts of the cluster's glyphs, or the style's font when it
  // has none.
  var clusterMetrics = Metrics(ascent: 0, descent: 0, leading: 0)
  for font in clusterFonts {
    let candidate = font.metrics
    clusterMetrics = Metrics(
      ascent: max(clusterMetrics.ascent, candidate.ascent),
      descent: max(clusterMetrics.descent, candidate.descent),
      leading: max(clusterMetrics.leading, candidate.leading)
    )
  }

  let fontMetrics = clusterMetrics
  clusterMetrics = Metrics(ascent: max(0, fontMetrics.ascent + style.baselineOffset), descent: max(0, fontMetrics.descent - style.baselineOffset), leading: fontMetrics.leading)

  var inkRect = CGRect.null
  for record in records {
    let bounds = record.bounds
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
      x: 0, y: -fontMetrics.ascent - style.baselineOffset, width: advance,
      height: fontMetrics.ascent + fontMetrics.descent)
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

  // A cluster's records are in shaped order, so each run's glyphs are adjacent.
  let strokeWidth = style.syntheticBold ? style.size * 0.03 : 0
  var signatureSize = 41 + (style.language.map(SignatureWriter.size(of:)) ?? 0)
  forEachGlyphBatch(records) { batch in
    signatureSize += SignatureWriter.size(of: fonts[batch.first!.font].postScriptName) + 8 + batch.count * 16
  }
  let (signature, renderIdentifier) = signatures.append(count: signatureSize) { writer in
    writer.write(globalEnd - globalStart)
    writer.write(request.metrics_generation)
    writer.write(Float(advance).bitPattern)
    writer.write(UInt64(bidiLevel))
    // Identical whitespace glyphs can carry distinct marker inheritance even
    // when their shaping geometry is equal (for example, language alone).
    writer.write(Float(style.letterSpacing).bitPattern)
    writer.write(Float(style.baselineOffset).bitPattern)
    writer.write(style.direction)
    writer.write(UInt8(style.language == nil ? 0 : 1))
    if let language = style.language { writer.write(language) }
    forEachGlyphBatch(records) { batch in
      let font = fonts[batch.first!.font]
      writer.write(font.postScriptName)
      writer.write(Float(strokeWidth).bitPattern)
      writer.write(Float(font.obliqueness).bitPattern)
      for record in batch {
        writer.write(UInt64(record.glyph))
        writer.write(Float(record.position.x - visualLeft).bitPattern)
        writer.write(Float(record.position.y).bitPattern)
      }
    }
  }
  var fallback = clusterFonts[0].postScriptName
  if clusterFonts.count > 1 {
    var names: [String] = []
    for font in clusterFonts where !names.contains(font.postScriptName) { names.append(font.postScriptName) }
    fallback = names.joined(separator: ", ")
  }

  return ClusterResult(
    start: globalStart,
    end: globalEnd,
    advance: advance,
    metrics: clusterMetrics,
    typographicBounds: Bounds(
      x: 0,
      y: -fontMetrics.ascent - style.baselineOffset,
      width: advance,
      height: fontMetrics.ascent + fontMetrics.descent
    ),
    inkBounds: Bounds(
      x: inkRect.minX,
      y: inkRect.minY,
      width: inkRect.width,
      height: inkRect.height
    ),
    bidiLevel: bidiLevel,
    fallbackFont: fallback.isEmpty ? CoreTextMeasurementProvider.defaultFontFamily : fallback,
    startCaret: startCaret,
    endCaret: endCaret,
    style: style,
    recordRange: recordRange,
    visualLeft: visualLeft,
    isColorGlyph: clusterFonts.contains { $0.isColorGlyph },
    signature: signature,
    renderIdentifier: renderIdentifier
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

  /// Clusters repeat the same few font names; responses share one copy.
  private var storedStrings: [String: ViemUtf8Slice] = [:]

  func storeUTF8(_ value: String) -> ViemUtf8Slice {
    if let stored = storedStrings[value] { return stored }
    let slice = copyUTF8(value)
    storedStrings[value] = slice
    return slice
  }

  private func copyUTF8(_ value: String) -> ViemUtf8Slice {
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
