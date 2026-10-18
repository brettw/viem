import CoreGraphics
import Darwin
import Foundation
import ImageIO

/// Immutable passive raster data. Decoders only receive local bytes.
struct CoreTextInlineImage: @unchecked Sendable {
  enum State {
    /// `measured`: intrinsic dimensions are known from the file header, but no
    /// bitmap is decoded yet. Layout uses them exactly, like `ready`.
    case pending, measured, ready, remote, broken, limited
    var isTerminalPlaceholder: Bool { self == .broken || self == .limited }
    var hasExactDimensions: Bool { self == .ready || self == .measured }
  }
  let location: String
  let image: CGImage?
  let size: CGSize
  let identity: UUID
  let state: State
  var cost: Int { image.map { $0.bytesPerRow * $0.height } ?? 0 }

  init(location: String, image: CGImage?, size: CGSize, identity: UUID, state: State? = nil) {
    self.location = location; self.image = image; self.size = size; self.identity = identity
    self.state = state ?? (image == nil ? .pending : .ready)
  }
  static func placeholder(_ location: String, state: State = .pending) -> Self {
    Self(location: location, image: nil, size: CGSize(width: 300, height: 64), identity: UUID(), state: state)
  }
  /// Geometry used by layout: a change requires reshaping, while anything
  /// else (a decoded bitmap, a broken indicator) is repainted in place.
  var layoutKey: CoreTextImageLayoutKey { CoreTextImageLayoutKey(size: size, exact: state.hasExactDimensions) }
  /// Render resources reference the bitmap through the shared resources at
  /// draw time, so retained geometry never pins rasters past their budget.
  func withoutRaster() -> Self {
    Self(location: location, image: nil, size: size, identity: identity, state: state)
  }
}

struct CoreTextImageLayoutKey: Equatable {
  let size: CGSize
  let exact: Bool
}

/// Bitmap eviction never forgets accepted intrinsic dimensions. Small immutable
/// metadata stays for the document lifetime, separately from the raster budget.
final class CoreTextImageResources: @unchecked Sendable {
  private struct Knowledge {
    var size = CGSize(width: 300, height: 64)
    var state = CoreTextInlineImage.State.pending
    var identity = UUID()
    var version = UUID()
    /// Whether layout has used these dimensions since they were recorded. A
    /// dimension change is a geometry change only for shaped layout.
    var served = false
    func placeholder(_ location: String) -> CoreTextInlineImage {
      CoreTextInlineImage(location: location, image: nil, size: size, identity: identity, state: state)
    }
    var layoutKey: CoreTextImageLayoutKey { CoreTextImageLayoutKey(size: size, exact: state.hasExactDimensions) }
  }
  private let lock = NSLock()
  private let queue: OperationQueue = {
    let queue = OperationQueue(); queue.name = "Viem local image previews"
    queue.maxConcurrentOperationCount = 2; queue.qualityOfService = .userInitiated
    return queue
  }()
  private static let maximumEntries = 64
  private static let maximumBytes = 64 * 1024 * 1024
  private static let maximumPending = 16
  private static let maximumMetadataEntries = 65_536
  private static let maximumMetadataBytes = 32 * 1024 * 1024
  // Preview admission uses decimal MB and source pixels, before decoding.
  private static let maximumFileBytes = 10_000_000
  private static let maximumPixelDimension = 5_000
  private let metadataByteBudget: Int
  private let decoder: @Sendable (URL, String) -> CoreTextInlineImage?
  private let prober: @Sendable (URL, String) -> CoreTextInlineImage?
  private var baseURL: URL?
  private var epoch = UUID()
  private var entries: [String: CoreTextInlineImage] = [:]
  private var known: [String: Knowledge] = [:]
  private var metadataBytes = 0
  private var pending: [UUID: (epoch: UUID, location: String, version: UUID)] = [:]
  private var order: [String] = []
  private var bytes = 0
  private var changed: (@Sendable () -> Void)?
  private var resourcesChanged: (@Sendable ([String]) -> Void)?
  private var rastersChanged: (@Sendable () -> Void)?
  private var stopped = false
  private var needsRetry = false
  private var visible: Set<String>?
  private var viewportID: UInt64?
  private var admitted = Set<String>()
  private var attempted = Set<String>()
  private var reloadRequested = Set<String>()
  private var loads = 0
  private var probes = 0
  private struct DimensionRequest {
    let location: String
    let epoch: UUID
    let cancellation: Operation
    let completion: @MainActor @Sendable (CGSize?) -> Void
  }
  private var dimensionRequest: DimensionRequest?
  private var dimensionRequestRunning = false
  /// Bitmaps resampled to the device size they are drawn at, so scrolling
  /// draws them nearly 1:1 instead of resampling a large raster every frame.
  private struct Scaled { let source: CGImage; let pixels: CGSize; let image: CGImage }
  private var scaled: [String: Scaled] = [:]
  private var scaledOrder: [String] = []
  private var scaledBytes = 0
  private static let maximumScaledEntries = 16
  private static let maximumScaledBytes = 48 * 1024 * 1024
  private var scaledRenders = 0
  private var scaling: [String: (source: CGImage, pixels: CGSize)] = [:]

  init(metadataByteBudget: Int = 32 * 1024 * 1024,
    decoder: @escaping @Sendable (URL, String) -> CoreTextInlineImage? = {
      CoreTextImageResources.decode($0, location: $1)
    },
    prober: @escaping @Sendable (URL, String) -> CoreTextInlineImage? = {
      CoreTextImageResources.decode($0, location: $1, dimensionsOnly: true)
    }) {
    self.metadataByteBudget = max(0, min(metadataByteBudget, Self.maximumMetadataBytes))
    self.decoder = decoder
    self.prober = prober
  }

  var localLoadCount: Int { lock.lock(); defer { lock.unlock() }; return loads }
  var dimensionProbeCount: Int { lock.lock(); defer { lock.unlock() }; return probes }
  var scaledRenderCount: Int { lock.lock(); defer { lock.unlock() }; return scaledRenders }
  var pendingLoadCount: Int { lock.lock(); defer { lock.unlock() }; return pending.count }
  var retainedRasterBytes: Int { lock.lock(); defer { lock.unlock() }; return bytes }
  var retainedEntryCount: Int { lock.lock(); defer { lock.unlock() }; return entries.count }
  func setChangeHandler(_ handler: (@Sendable () -> Void)?) { lock.lock(); changed = handler; lock.unlock() }
  /// Called on the main thread with locations whose layout geometry changed
  /// after layout used their previous dimensions.
  func setResourceChangeHandler(_ handler: (@Sendable ([String]) -> Void)?) { lock.lock(); resourcesChanged = handler; lock.unlock() }
  /// Called on the main thread when bitmaps or placeholder states changed
  /// without changing any geometry layout has used: repaint only.
  func setRasterChangeHandler(_ handler: (@Sendable () -> Void)?) { lock.lock(); rastersChanged = handler; lock.unlock() }

  private func pendingCurrent(_ location: String) -> Bool {
    pending.values.contains { $0.epoch == epoch && $0.location == location && $0.version == known[location]?.version }
  }
  private func remember(_ location: String) -> Bool {
    if known[location] != nil { return true }
    let cost = location.utf8.count + 128
    guard known.count < Self.maximumMetadataEntries, metadataBytes + cost <= metadataByteBudget else { return false }
    known[location] = Knowledge(); metadataBytes += cost
    return true
  }

  /// New viewports restart texture admission, never the dimension metadata.
  @discardableResult
  func setVisibleLocations(_ locations: [String], viewportID nextID: UInt64 = 0) -> Bool {
    lock.lock()
    var ordered: [String] = []; var next = Set<String>()
    for location in locations.prefix(256) where next.count < Self.maximumEntries {
      guard location.utf8.count <= 8192, Self.localURL(location, relativeTo: baseURL) != nil,
            next.insert(location).inserted else { continue }
      ordered.append(location)
    }
    guard !stopped else { lock.unlock(); return false }
    if viewportID != nextID {
      viewportID = nextID; admitted = next; attempted.removeAll(); needsRetry = false
    } else {
      for location in ordered where admitted.count < Self.maximumEntries { admitted.insert(location) }
    }
    for location in next where admitted.contains(location) {
      if !reloadRequested.contains(location), entries[location]?.image != nil || known[location]?.state.isTerminalPlaceholder == true || pendingCurrent(location) {
        attempted.insert(location)
      }
    }
    visible = next
    let retry = next.contains { admitted.contains($0) && !attempted.contains($0) }
    let capacity = pending.count < Self.maximumPending
    if retry && !capacity { needsRetry = true }
    let start = ordered.filter { admitted.contains($0) && !attempted.contains($0) }
    lock.unlock()
    // Admission starts decoding directly; cached geometry need not be shaped
    // again merely to request a bitmap.
    for location in start { _ = image(for: location, servesLayout: false) }
    return retry && capacity
  }

  @discardableResult
  func setDocumentURL(_ url: URL?) -> Bool {
    lock.lock(); defer { lock.unlock() }
    guard baseURL != url else { return false }
    baseURL = url; epoch = UUID()
    entries.removeAll(); order.removeAll(); bytes = 0
    scaled.removeAll(); scaledOrder.removeAll(); scaledBytes = 0; scaling.removeAll()
    known.removeAll(); metadataBytes = 0; reloadRequested.removeAll()
    dimensionRequest = nil
    visible = nil; viewportID = nil; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    // Keep old jobs counted until they drain; never cancel a new epoch's job.
    return true
  }

  func stop() {
    lock.lock()
    stopped = true; epoch = UUID(); changed = nil; resourcesChanged = nil; rastersChanged = nil
    dimensionRequest = nil
    entries.removeAll(); known.removeAll(); metadataBytes = 0
    scaled.removeAll(); scaledOrder.removeAll(); scaledBytes = 0; scaling.removeAll()
    pending.removeAll(); order.removeAll(); bytes = 0; reloadRequested.removeAll()
    visible = []; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    lock.unlock(); queue.cancelAllOperations()
  }

  func canReload(_ location: String) -> Bool {
    lock.lock(); defer { lock.unlock() }
    return !stopped && Self.localURL(location, relativeTo: baseURL) != nil
  }

  /// A cancelled popup probe cannot install into a later document or draft.
  /// It shares the bounded local decoder and reads headers without pixels.
  func requestDimensions(_ location: String, completion: @escaping @MainActor @Sendable (CGSize?) -> Void) -> Operation {
    let operation = BlockOperation()
    lock.lock()
    guard !stopped else { lock.unlock(); operation.cancel(); return operation }
    let superseded = dimensionRequest?.cancellation
    dimensionRequest = DimensionRequest(location: location, epoch: epoch, cancellation: operation, completion: completion)
    let start = !dimensionRequestRunning
    dimensionRequestRunning = true
    lock.unlock()
    superseded?.cancel()
    // One header read and one coalesced request, even if the filesystem stalls
    // while the user types or opens successive popups.
    if start { queue.addOperation { [weak self] in self?.runDimensionRequests() } }
    return operation
  }

  private func runDimensionRequests() {
    while true {
      lock.lock()
      guard !stopped, let request = dimensionRequest else {
        dimensionRequestRunning = false; lock.unlock(); return
      }
      dimensionRequest = nil
      lock.unlock()
      guard !request.cancellation.isCancelled else { continue }
      let value = image(for: request.location, probe: true, servesLayout: false, expectedEpoch: request.epoch)
      let size = value.state.hasExactDimensions ? value.size : nil
      DispatchQueue.main.async { [weak self] in
        guard let self, !request.cancellation.isCancelled else { return }
        self.lock.lock()
        let current = !self.stopped && self.epoch == request.epoch
          && (value.state == .remote || self.known[request.location]?.layoutKey == value.layoutKey)
        self.lock.unlock()
        if current { request.completion(size) }
      }
    }
  }

  @discardableResult
  func reload(_ location: String) -> Bool {
    lock.lock()
    guard !stopped, location.utf8.count <= 8192,
          Self.localURL(location, relativeTo: baseURL) != nil, remember(location) else { lock.unlock(); return false }
    // Keep the accepted bitmap and size while the replacement is decoded.
    // Its version rejects an older in-flight read, including repeated reloads.
    known[location]!.version = UUID()
    reloadRequested.insert(location); attempted.remove(location)
    lock.unlock()
    _ = image(for: location)
    return true
  }

  /// Dimensions and the best available bitmap for layout. A `probe` caller
  /// (a background layout worker) reads unknown dimensions synchronously
  /// from the file header so its geometry is final; the main thread never
  /// touches the filesystem and uses a placeholder until a load completes.
  func image(for location: String, probe: Bool = false) -> CoreTextInlineImage {
    image(for: location, probe: probe, servesLayout: true)
  }

  /// `servesLayout` is false for requests that only start a bitmap load.
  private func image(for location: String, probe: Bool = false, servesLayout: Bool, expectedEpoch: UUID? = nil) -> CoreTextInlineImage {
    guard location.utf8.count <= 8192 else { return .placeholder(location) }
    lock.lock()
    if let expectedEpoch, expectedEpoch != epoch || stopped {
      lock.unlock(); return .placeholder(location)
    }
    guard let local = Self.localURL(location, relativeTo: baseURL) else {
      lock.unlock(); return .placeholder(location, state: .remote)
    }
    if probe, !stopped, known[location].map({ $0.state == .pending }) ?? true,
       !reloadRequested.contains(location), remember(location) {
      let capturedEpoch = epoch; let version = known[location]!.version
      probes += 1
      lock.unlock()
      let measured = prober(local, location)
      lock.lock()
      if epoch == capturedEpoch, !stopped, known[location]?.version == version, known[location]?.state == .pending {
        record(measured ?? .placeholder(location, state: .broken), for: location, notifyFromWorker: true)
      }
    }
    // Layout may encounter a newly inserted location before the next visible
    // frame admits it for decoding. Remember that placeholder now so the load
    // invalidates its geometry instead of merely repainting stale bounds.
    if servesLayout { _ = remember(location) }
    let cached = entries[location]
    let result = cached ?? known[location]?.placeholder(location) ?? .placeholder(location)
    if servesLayout { known[location]?.served = true }
    // A worker only needs dimensions; bitmaps load for visible admission.
    if probe { lock.unlock(); return result }
    let force = reloadRequested.contains(location)
    if !force, cached?.image != nil || known[location]?.state.isTerminalPlaceholder == true {
      if admitted.contains(location) || (viewportID == nil && attempted.count < Self.maximumEntries) { attempted.insert(location) }
      lock.unlock(); return result
    }
    guard !stopped, !pendingCurrent(location),
          force || (visible?.contains(location) != false && (viewportID == nil || admitted.contains(location))
            && !attempted.contains(location) && attempted.count < Self.maximumEntries) else {
      lock.unlock(); return result
    }
    guard remember(location) else {
      if admitted.contains(location) || attempted.count < Self.maximumEntries { attempted.insert(location) }
      reloadRequested.remove(location)
      lock.unlock(); return result
    }
    guard pending.count < Self.maximumPending else { needsRetry = true; lock.unlock(); return result }
    let capturedEpoch = epoch; let version = known[location]!.version; let token = UUID()
    attempted.insert(location); reloadRequested.remove(location)
    pending[token] = (capturedEpoch, location, version); loads += 1
    lock.unlock()
    queue.addOperation { [weak self] in
      guard let self else { return }
      self.lock.lock()
      let current = self.epoch == capturedEpoch && !self.stopped && self.known[location]?.version == version
      self.lock.unlock()
      let decoded = current ? self.decoder(local, location) : nil
      DispatchQueue.main.async { [weak self] in
        self?.finish(token: token, epoch: capturedEpoch, location: location, version: version, decoded: decoded)
      }
    }
    return result
  }

  /// The current bitmap and placeholder state for drawing `captured`, when
  /// its geometry still matches what layout used. Never touches the disk.
  func presentation(matching captured: CoreTextInlineImage) -> CoreTextInlineImage? {
    lock.lock(); defer { lock.unlock() }
    guard !stopped else { return nil }
    let current = entries[captured.location] ?? known[captured.location]?.placeholder(captured.location)
    return current.flatMap { $0.layoutKey == captured.layoutKey ? $0 : nil }
  }

  /// `image`'s bitmap resampled to `pixels`, created once per bitmap and size
  /// on a background queue. Until it is ready, and for near-native or very
  /// large sizes, drawing uses the original; completion requests a repaint.
  func displayRaster(for image: CoreTextInlineImage, pixels: CGSize) -> CGImage? {
    guard let raster = image.image else { return nil }
    let width = Int(pixels.width.rounded()), height = Int(pixels.height.rounded())
    let differs = abs(width - raster.width) * 5 > raster.width || abs(height - raster.height) * 5 > raster.height
    guard width > 0, height > 0, differs, width * height * 4 <= Self.maximumScaledBytes / 4 else { return raster }
    let size = CGSize(width: width, height: height)
    let location = image.location
    lock.lock()
    if let entry = scaled[location], entry.source === raster, entry.pixels == size {
      lock.unlock(); return entry.image
    }
    guard !stopped, scaling[location].map({ !($0.source === raster && $0.pixels == size) }) ?? true else {
      lock.unlock(); return raster
    }
    scaling[location] = (raster, size)
    let capturedEpoch = epoch
    lock.unlock()
    Self.resampling.async { [weak self] in
      let space = raster.colorSpace.flatMap { $0.model == .rgb ? $0 : nil } ?? CGColorSpace(name: CGColorSpace.sRGB)!
      let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      context?.interpolationQuality = .high
      context?.draw(raster, in: CGRect(origin: .zero, size: size))
      let resampled = context?.makeImage()
      DispatchQueue.main.async { [weak self] in
        self?.finishResample(location: location, source: raster, pixels: size, image: resampled, epoch: capturedEpoch)
      }
    }
    return raster
  }

  private static let resampling = DispatchQueue(label: "Viem image resampling", qos: .utility)

  private func finishResample(location: String, source: CGImage, pixels: CGSize, image: CGImage?, epoch capturedEpoch: UUID) {
    lock.lock()
    if let pending = scaling[location], pending.source === source, pending.pixels == pixels {
      scaling.removeValue(forKey: location)
    }
    guard let image, !stopped, epoch == capturedEpoch,
          entries[location]?.image === source || known[location] != nil else { lock.unlock(); return }
    let cost = image.bytesPerRow * image.height
    if let previous = scaled.removeValue(forKey: location) {
      scaledBytes -= previous.image.bytesPerRow * previous.image.height
      scaledOrder.removeAll { $0 == location }
    }
    while (scaledOrder.count >= Self.maximumScaledEntries || scaledBytes + cost > Self.maximumScaledBytes),
          let oldest = scaledOrder.first {
      scaledOrder.removeFirst()
      if let evicted = scaled.removeValue(forKey: oldest) { scaledBytes -= evicted.image.bytesPerRow * evicted.image.height }
    }
    scaled[location] = Scaled(source: source, pixels: pixels, image: image)
    scaledOrder.append(location); scaledBytes += cost; scaledRenders += 1
    let repaint = rastersChanged
    lock.unlock()
    repaint?()
  }

  /// Record new knowledge under the lock. A geometry change is reported only
  /// if layout used the previous dimensions; otherwise a repaint suffices.
  /// Returns whether geometry changed.
  @discardableResult
  private func record(_ value: CoreTextInlineImage, for location: String, notifyFromWorker: Bool) -> Bool {
    let previous = known[location]!
    let geometryChanged = previous.served && previous.layoutKey != value.layoutKey
    known[location]!.size = value.size; known[location]!.state = value.state
    if previous.layoutKey != value.layoutKey {
      // Render resources compare identities; same geometry keeps its identity.
      known[location]!.identity = value.identity; known[location]!.served = false
    }
    if notifyFromWorker, geometryChanged, let handler = resourcesChanged {
      DispatchQueue.main.async { handler([location]) }
    }
    return geometryChanged
  }

  private func finish(token: UUID, epoch capturedEpoch: UUID, location: String, version: UUID, decoded: CoreTextInlineImage?) {
    lock.lock()
    guard pending.removeValue(forKey: token) != nil, !stopped else { lock.unlock(); return }
    var published = false
    var geometryChanged = false
    if epoch == capturedEpoch, known[location]?.version == version {
      // Record dimensions even if this image scrolled offscreen while loading.
      let value = decoded ?? .placeholder(location, state: .broken)
      geometryChanged = record(value, for: location, notifyFromWorker: false)
      if let previous = entries.removeValue(forKey: location) { bytes -= previous.cost }
      order.removeAll { $0 == location }
      if value.image != nil, visible?.contains(location) != false {
        while (bytes + value.cost > Self.maximumBytes || order.count >= Self.maximumEntries),
              let oldest = order.first(where: { visible?.contains($0) != true }) {
          order.removeAll { $0 == oldest }
          if let previous = entries.removeValue(forKey: oldest) { bytes -= previous.cost }
        }
        if bytes + value.cost <= Self.maximumBytes && order.count < Self.maximumEntries {
          entries[location] = CoreTextInlineImage(location: location, image: value.image, size: value.size,
            identity: known[location]!.identity, state: value.state)
          order.append(location); bytes += value.cost
        }
      }
      published = true
    }
    let retry = needsRetry; needsRetry = false
    let handler = published || retry ? changed : nil
    let resourceHandler = geometryChanged ? resourcesChanged : nil
    let rasterHandler = published && !geometryChanged ? rastersChanged : nil
    let reloads = reloadRequested
    let admittedRetry = retry ? (visible ?? []).filter { admitted.contains($0) && !attempted.contains($0) } : []
    lock.unlock()
    handler?(); resourceHandler?([location]); rasterHandler?()
    // Explicit reloads must progress even when their old request occupied the
    // last queue slot; they do not depend on another caret or wheel event.
    for location in reloads { _ = image(for: location, servesLayout: false) }
    for location in admittedRetry { _ = image(for: location, servesLayout: false) }
  }

  /// Pure lexical resolution: remote schemes, hosts and protocol-relative URLs
  /// fail before any filesystem API is consulted.
  static func localURL(_ location: String, relativeTo base: URL?) -> URL? {
    guard !location.isEmpty, !location.contains(where: { $0.isNewline || $0.asciiValue.map { $0 < 32 || $0 == 127 } == true }),
          !location.hasPrefix("//"), !location.hasPrefix("\\\\"),
          let parts = URLComponents(string: location),
          parts.scheme == nil || parts.scheme?.lowercased() == "file",
          parts.host == nil || parts.host == "" || parts.host?.lowercased() == "localhost",
          parts.query == nil, parts.user == nil, parts.password == nil,
          let path = parts.percentEncodedPath.removingPercentEncoding,
          !path.contains(where: { $0.unicodeScalars.contains { $0.value < 32 || $0.value == 127 } }),
          !path.hasPrefix("//") else { return nil }
    let result: URL
    if path.hasPrefix("/") { result = URL(fileURLWithPath: path) }
    else {
      guard parts.scheme == nil, let base, base.isFileURL else { return nil }
      result = base.deletingLastPathComponent().appendingPathComponent(path)
    }
    return result.standardizedFileURL
  }

  /// With `dimensionsOnly`, reads the verified file and its header metadata
  /// but decodes no pixels, returning a `measured` image.
  static func decode(_ url: URL, location: String, dimensionsOnly: Bool = false) -> CoreTextInlineImage? {
    guard url.isFileURL else { return nil }
    // Resolving symlinks is a native file operation. A nonlocal volume, special
    // file or excessive resource is rejected before reading image contents.
    let file = url.resolvingSymlinksInPath()
    var status = stat()
    guard lstat(file.path, &status) == 0, status.st_flags & UInt32(SF_DATALESS) == 0 else { return nil }
    let keys: Set<URLResourceKey> = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey, .volumeIsLocalKey,
        .isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey]
    guard let before = try? file.resourceValues(forKeys: keys), before.isRegularFile == true,
          before.volumeIsLocal == true,
          before.isUbiquitousItem != true || before.ubiquitousItemDownloadingStatus == .current || before.ubiquitousItemDownloadingStatus == .downloaded,
          let size = before.fileSize, size > 0 else { return nil }
    guard size <= maximumFileBytes else { return .placeholder(location, state: .limited) }
    let descriptor = open(file.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
    guard descriptor >= 0 else { return nil }
    let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    defer { try? handle.close() }
    var opened = stat(); var finished = stat(); var current = stat(); var volume = statfs()
    guard fstat(descriptor, &opened) == 0, opened.st_mode & S_IFMT == S_IFREG,
          fstatfs(descriptor, &volume) == 0, volume.f_flags & UInt32(MNT_LOCAL) != 0,
          opened.st_dev == status.st_dev, opened.st_ino == status.st_ino,
          opened.st_size == status.st_size,
          opened.st_mtimespec.tv_sec == status.st_mtimespec.tv_sec,
          opened.st_mtimespec.tv_nsec == status.st_mtimespec.tv_nsec,
          opened.st_flags & UInt32(SF_DATALESS) == 0, opened.st_size == size,
          let data = try? handle.read(upToCount: maximumFileBytes + 1), data.count == size,
          fstat(descriptor, &finished) == 0, lstat(file.path, &current) == 0,
          opened.st_ino == current.st_ino, opened.st_dev == current.st_dev,
          opened.st_size == finished.st_size, opened.st_size == current.st_size,
          opened.st_mtimespec.tv_sec == finished.st_mtimespec.tv_sec,
          opened.st_mtimespec.tv_nsec == finished.st_mtimespec.tv_nsec,
          opened.st_mtimespec.tv_sec == current.st_mtimespec.tv_sec,
          opened.st_mtimespec.tv_nsec == current.st_mtimespec.tv_nsec,
          let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
          let type = CGImageSourceGetType(source) as String?,
          ["public.png", "public.jpeg", "com.compuserve.gif", "public.tiff", "com.microsoft.bmp", "org.webmproject.webp", "public.heic", "public.heif"].contains(type),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
          let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
          width.doubleValue > 0, height.doubleValue > 0 else { return nil }
    guard width.doubleValue <= Double(maximumPixelDimension), height.doubleValue <= Double(maximumPixelDimension)
      else { return .placeholder(location, state: .limited) }
    if dimensionsOnly {
      let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
      let transpose = (5...8).contains(orientation)
      return CoreTextInlineImage(location: location, image: nil,
        size: CGSize(width: transpose ? height.doubleValue : width.doubleValue,
                     height: transpose ? width.doubleValue : height.doubleValue), identity: UUID(), state: .measured)
    }
    let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 2048,
      kCGImageSourceShouldCacheImmediately: true]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
    let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
    let transpose = (5...8).contains(orientation)
    return CoreTextInlineImage(location: location, image: image,
      size: CGSize(width: transpose ? height.doubleValue : width.doubleValue,
                   height: transpose ? width.doubleValue : height.doubleValue), identity: UUID())
  }
}
