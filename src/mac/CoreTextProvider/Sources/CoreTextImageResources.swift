import CoreGraphics
import Darwin
import Foundation
import ImageIO

/// Immutable, passive raster data. ImageIO only receives already-read local bytes;
/// URLs (including URLs embedded in SVG) are never passed to an image decoder.
struct CoreTextInlineImage: @unchecked Sendable {
  let location: String
  let image: CGImage?
  let size: CGSize
  let identity: UUID
  var cost: Int { image.map { $0.bytesPerRow * $0.height } ?? 0 }

  static func placeholder(_ location: String) -> Self {
    Self(location: location, image: nil, size: CGSize(width: 300, height: 64), identity: UUID())
  }
}

/// A view and its immutable worker providers share this bounded cache. Loading
/// never blocks shaping. A completed local preview publishes a metrics change.
final class CoreTextImageResources: @unchecked Sendable {
  private let lock = NSLock()
  private let queue: OperationQueue = {
    let queue = OperationQueue()
    queue.name = "Viem local image previews"
    queue.maxConcurrentOperationCount = 2
    queue.qualityOfService = .userInitiated
    return queue
  }()
  private static let maximumEntries = 64
  private static let maximumBytes = 64 * 1024 * 1024
  private static let maximumPending = 16
  private let decoder: @Sendable (URL, String) -> CoreTextInlineImage?
  private var baseURL: URL?
  private var epoch = UUID()
  private var entries: [String: CoreTextInlineImage] = [:]
  // Tokens stay counted across URL epochs: changing a document URL cannot
  // create another queue or exceed the two-worker / sixteen-job budget.
  private var pending: [UUID: (epoch: UUID, location: String)] = [:]
  private var order: [String] = []
  private var bytes = 0
  private var changed: (@Sendable () -> Void)?
  private var stopped = false
  private var needsRetry = false
  private var visible: Set<String>?
  private var viewportID: UInt64?
  private var admitted = Set<String>()
  private var attempted = Set<String>()
  private var loads = 0

  init(decoder: @escaping @Sendable (URL, String) -> CoreTextInlineImage? = {
    CoreTextImageResources.decode($0, location: $1)
  }) {
    self.decoder = decoder
  }

  var localLoadCount: Int { lock.lock(); defer { lock.unlock() }; return loads }
  var pendingLoadCount: Int { lock.lock(); defer { lock.unlock() }; return pending.count }
  var retainedRasterBytes: Int { lock.lock(); defer { lock.unlock() }; return bytes }
  var retainedEntryCount: Int { lock.lock(); defer { lock.unlock() }; return entries.count }

  func setChangeHandler(_ handler: (@Sendable () -> Void)?) {
    lock.lock(); changed = handler; lock.unlock()
  }

  /// The caller advances viewportID only for user viewport, source or document
  /// changes. Image-driven reflow can reveal more locations within that ID,
  /// but cannot reset its finite admission/attempt budget.
  /// True means admitted visible placeholders need a fresh shaping attempt.
  @discardableResult
  func setVisibleLocations(_ locations: [String], viewportID nextID: UInt64 = 0) -> Bool {
    lock.lock()
    var ordered: [String] = []
    var next = Set<String>()
    for location in locations.prefix(256) where next.count < Self.maximumEntries {
      guard location.utf8.count <= 8192, Self.localURL(location, relativeTo: baseURL) != nil,
            next.insert(location).inserted else { continue }
      ordered.append(location)
    }
    guard !stopped else { lock.unlock(); return false }
    if viewportID != nextID {
      viewportID = nextID
      admitted = next
      attempted = Set(next.filter { location in entries[location]?.image != nil
        || pending.values.contains(where: { $0.epoch == epoch && $0.location == location }) })
      needsRetry = false
    } else {
      for location in ordered where admitted.count < Self.maximumEntries { admitted.insert(location) }
    }
    for location in next where admitted.contains(location) {
      if entries[location]?.image != nil
        || pending.values.contains(where: { $0.epoch == epoch && $0.location == location }) {
        attempted.insert(location)
      }
    }
    visible = next
    let retry = next.contains { admitted.contains($0) && !attempted.contains($0) }
    let hasCapacity = pending.count < Self.maximumPending
    if retry && !hasCapacity { needsRetry = true }
    lock.unlock()
    return retry && hasCapacity
  }

  @discardableResult
  func setDocumentURL(_ url: URL?) -> Bool {
    lock.lock()
    guard baseURL != url else { lock.unlock(); return false }
    baseURL = url
    epoch = UUID()
    entries.removeAll(); order.removeAll(); bytes = 0
    visible = nil; viewportID = nil; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    lock.unlock()
    // Do not cancel the shared queue after publishing a new epoch: a worker
    // could already have queued new work. Old jobs validate their captured
    // epoch before decoding and always relinquish their pending token.
    return true
  }

  func stop() {
    lock.lock()
    stopped = true; epoch = UUID(); changed = nil
    entries.removeAll(); pending.removeAll(); order.removeAll(); bytes = 0
    visible = []; admitted.removeAll(); attempted.removeAll(); needsRetry = false
    lock.unlock()
    queue.cancelAllOperations()
  }

  func image(for location: String) -> CoreTextInlineImage {
    guard location.utf8.count <= 8192 else { return .placeholder(location) }
    lock.lock()
    let cached = entries[location]
    if let cached, cached.image != nil {
      if admitted.contains(location) || (viewportID == nil && attempted.count < Self.maximumEntries) { attempted.insert(location) }
      lock.unlock(); return cached
    }
    let placeholder = cached ?? CoreTextInlineImage.placeholder(location)
    guard !stopped, visible?.contains(location) != false,
          viewportID == nil || admitted.contains(location),
          !attempted.contains(location), attempted.count < Self.maximumEntries,
          let local = Self.localURL(location, relativeTo: baseURL) else {
      lock.unlock(); return placeholder
    }
    guard pending.count < Self.maximumPending else {
      needsRetry = true; lock.unlock(); return placeholder
    }
    if cached == nil {
      while order.count >= Self.maximumEntries, let oldest = order.first(where: { visible?.contains($0) != true }) ?? order.first {
        order.removeAll { $0 == oldest }
        if let previous = entries.removeValue(forKey: oldest) { bytes -= previous.cost }
      }
      entries[location] = placeholder; order.append(location)
    }
    let capturedEpoch = epoch
    let token = UUID()
    attempted.insert(location)
    pending[token] = (capturedEpoch, location); loads += 1
    lock.unlock()
    queue.addOperation { [weak self] in
      guard let self else { return }
      self.lock.lock()
      let current = self.epoch == capturedEpoch && !self.stopped && self.pending[token] != nil
      self.lock.unlock()
      let decoded = current ? self.decoder(local, location) : nil
      DispatchQueue.main.async { [weak self] in
        self?.finish(token: token, epoch: capturedEpoch, location: location,
          placeholder: placeholder, decoded: decoded)
      }
    }
    return placeholder
  }

  private func finish(token: UUID, epoch capturedEpoch: UUID, location: String,
    placeholder: CoreTextInlineImage, decoded: CoreTextInlineImage?) {
    lock.lock()
    guard pending.removeValue(forKey: token) != nil, !stopped else { lock.unlock(); return }
    var installed = false
    if epoch == capturedEpoch, visible?.contains(location) != false,
       entries[location]?.identity == placeholder.identity,
       let decoded, decoded.cost <= Self.maximumBytes {
      while bytes + decoded.cost > Self.maximumBytes,
            let oldest = order.first(where: { $0 != location && visible?.contains($0) != true }) {
        order.removeAll { $0 == oldest }
        if let previous = entries.removeValue(forKey: oldest) { bytes -= previous.cost }
      }
      // Retain already-visible previews when their working set fills the
      // budget. The declined location stays a stable, already-attempted
      // placeholder until a real viewport change makes room.
      if bytes + decoded.cost <= Self.maximumBytes {
        entries[location] = decoded; bytes += decoded.cost
        installed = true
      }
    }
    let handler = installed || needsRetry ? changed : nil
    needsRetry = false
    lock.unlock()
    handler?()
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
          let size = before.fileSize, size > 0, size <= 32 * 1024 * 1024 else { return nil }
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
          let data = try? handle.read(upToCount: 32 * 1024 * 1024 + 1), data.count == size,
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
          width.doubleValue > 0, height.doubleValue > 0,
          width.doubleValue <= 32768, height.doubleValue <= 32768,
          width.doubleValue * height.doubleValue <= 64 * 1024 * 1024 else { return nil }
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
