import CoreGraphics
import Darwin
import Foundation
import ImageIO

/// Immutable passive raster data. Decoders only receive local bytes.
struct CoreTextInlineImage: @unchecked Sendable {
  enum State {
    case pending, ready, remote, broken, limited
    var isTerminalPlaceholder: Bool { self == .broken || self == .limited }
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
}

/// Bitmap eviction never forgets accepted intrinsic dimensions. Small immutable
/// metadata stays for the document lifetime, separately from the raster budget.
final class CoreTextImageResources: @unchecked Sendable {
  private struct Knowledge {
    var size = CGSize(width: 300, height: 64)
    var state = CoreTextInlineImage.State.pending
    var identity = UUID()
    var version = UUID()
    func placeholder(_ location: String) -> CoreTextInlineImage {
      CoreTextInlineImage(location: location, image: nil, size: size, identity: identity, state: state)
    }
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
  private var stopped = false
  private var needsRetry = false
  private var visible: Set<String>?
  private var viewportID: UInt64?
  private var admitted = Set<String>()
  private var attempted = Set<String>()
  private var reloadRequested = Set<String>()
  private var loads = 0

  init(metadataByteBudget: Int = 32 * 1024 * 1024,
    decoder: @escaping @Sendable (URL, String) -> CoreTextInlineImage? = {
      CoreTextImageResources.decode($0, location: $1)
    }) {
    self.metadataByteBudget = max(0, min(metadataByteBudget, Self.maximumMetadataBytes))
    self.decoder = decoder
  }

  var localLoadCount: Int { lock.lock(); defer { lock.unlock() }; return loads }
  var pendingLoadCount: Int { lock.lock(); defer { lock.unlock() }; return pending.count }
  var retainedRasterBytes: Int { lock.lock(); defer { lock.unlock() }; return bytes }
  var retainedEntryCount: Int { lock.lock(); defer { lock.unlock() }; return entries.count }
  func setChangeHandler(_ handler: (@Sendable () -> Void)?) { lock.lock(); changed = handler; lock.unlock() }
  func setResourceChangeHandler(_ handler: (@Sendable ([String]) -> Void)?) { lock.lock(); resourcesChanged = handler; lock.unlock() }

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
    lock.unlock()
    return retry && capacity
  }

  @discardableResult
  func setDocumentURL(_ url: URL?) -> Bool {
    lock.lock(); defer { lock.unlock() }
    guard baseURL != url else { return false }
    baseURL = url; epoch = UUID()
    entries.removeAll(); order.removeAll(); bytes = 0
    known.removeAll(); metadataBytes = 0; reloadRequested.removeAll()
    visible = nil; viewportID = nil; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    // Keep old jobs counted until they drain; never cancel a new epoch's job.
    return true
  }

  func stop() {
    lock.lock()
    stopped = true; epoch = UUID(); changed = nil; resourcesChanged = nil
    entries.removeAll(); known.removeAll(); metadataBytes = 0
    pending.removeAll(); order.removeAll(); bytes = 0; reloadRequested.removeAll()
    visible = []; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    lock.unlock(); queue.cancelAllOperations()
  }

  func canReload(_ location: String) -> Bool {
    lock.lock(); defer { lock.unlock() }
    return !stopped && Self.localURL(location, relativeTo: baseURL) != nil
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

  func image(for location: String) -> CoreTextInlineImage {
    guard location.utf8.count <= 8192 else { return .placeholder(location) }
    lock.lock()
    guard let local = Self.localURL(location, relativeTo: baseURL) else {
      lock.unlock(); return .placeholder(location, state: .remote)
    }
    let cached = entries[location]
    let result = cached ?? known[location]?.placeholder(location) ?? .placeholder(location)
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

  private func finish(token: UUID, epoch capturedEpoch: UUID, location: String, version: UUID, decoded: CoreTextInlineImage?) {
    lock.lock()
    guard pending.removeValue(forKey: token) != nil, !stopped else { lock.unlock(); return }
    var published = false
    var geometryChanged = false
    if epoch == capturedEpoch, known[location]?.version == version {
      // Record dimensions even if this image scrolled offscreen while loading.
      let value = decoded ?? .placeholder(location, state: .broken)
      geometryChanged = known[location]!.size != value.size
      known[location]!.size = value.size; known[location]!.state = value.state; known[location]!.identity = value.identity
      if let previous = entries.removeValue(forKey: location) { bytes -= previous.cost }
      order.removeAll { $0 == location }
      if value.image != nil, visible?.contains(location) != false {
        while (bytes + value.cost > Self.maximumBytes || order.count >= Self.maximumEntries),
              let oldest = order.first(where: { visible?.contains($0) != true }) {
          order.removeAll { $0 == oldest }
          if let previous = entries.removeValue(forKey: oldest) { bytes -= previous.cost }
        }
        if bytes + value.cost <= Self.maximumBytes && order.count < Self.maximumEntries {
          entries[location] = value; order.append(location); bytes += value.cost
        }
      }
      published = true
    }
    let retry = needsRetry; needsRetry = false
    let handler = published || retry ? changed : nil
    let resourceHandler = published || retry ? resourcesChanged : nil
    let reloads = reloadRequested
    lock.unlock()
    handler?(); resourceHandler?(geometryChanged ? [location] : [])
    // Explicit reloads must progress even when their old request occupied the
    // last queue slot; they do not depend on another caret or wheel event.
    for location in reloads { _ = image(for: location) }
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

  static func decode(_ url: URL, location: String) -> CoreTextInlineImage? {
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
