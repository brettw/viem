import Darwin
import Foundation

/// Reports possible changes to one open document. Content comparison and the
/// decision to reload remain the document's responsibility.
@MainActor
final class EVFileChangeMonitor {
  private struct FileIdentity: Equatable {
    let device: dev_t
    let inode: ino_t

    init(_ info: stat) {
      device = info.st_dev
      inode = info.st_ino
    }
  }

  private struct DirectoryWatch {
    let source: DispatchSourceFileSystemObject
    let identity: FileIdentity
  }

  private let url: URL
  private let onChange: @MainActor () -> Void
  private var fileSource: DispatchSourceFileSystemObject?
  private var directoryWatches: [URL: DirectoryWatch] = [:]
  private var watchedFileIdentity: FileIdentity?
  private var pendingNotification: DispatchWorkItem?
  private var generation: UInt64 = 0
  private var isStopped = false

  init(url: URL, onChange: @escaping @MainActor () -> Void) {
    self.url = url.standardizedFileURL
    self.onChange = onChange
    // Attach directories first so a replacement during setup is still seen.
    refreshDirectoryWatches()
    refreshFileWatch()
  }

  func stop() {
    guard !isStopped else { return }
    isStopped = true
    generation &+= 1
    pendingNotification?.cancel()
    pendingNotification = nil
    fileSource?.cancel()
    fileSource = nil
    for watch in directoryWatches.values { watch.source.cancel() }
    directoryWatches.removeAll()
    watchedFileIdentity = nil
  }

  deinit {
    pendingNotification?.cancel()
    fileSource?.cancel()
    for watch in directoryWatches.values { watch.source.cancel() }
  }

  private func refreshDirectoryWatches() {
    let directories = watchedDirectories()
    // Register replacements and fallback ancestors before retiring sources.
    // A removed directory's old vnode cannot report recreation at its path.
    for directory in directories {
      let currentIdentity = identity(at: directory, directoryOnly: true)
      if let existing = directoryWatches[directory], existing.identity == currentIdentity { continue }
      guard currentIdentity != nil else { continue }
      var openedIdentity: FileIdentity?
      let source = makeSource(
        at: directory, events: [.write, .rename, .delete, .revoke],
        openedIdentity: { openedIdentity = $0 }
      ) { [weak self] in
        guard let self, !self.isStopped else { return }
        self.refreshDirectoryWatches()
        // A sibling being saved should not make us read this document again.
        if self.refreshFileWatch() { self.scheduleNotification() }
      }
      if let source, let openedIdentity {
        let previous = directoryWatches[directory]
        directoryWatches[directory] = DirectoryWatch(source: source, identity: openedIdentity)
        previous?.source.cancel()
      }
    }
    for directory in Array(directoryWatches.keys) where !directories.contains(directory) {
      directoryWatches.removeValue(forKey: directory)?.source.cancel()
    }
  }

  /// Keep the alias's parent and every target parent in a symbolic-link chain.
  /// Reading link destinations explicitly also works while the target is absent:
  /// resolving only an existing vnode would miss its later recreation elsewhere.
  private func watchedDirectories() -> Set<URL> {
    var directories = Set<URL>()
    var visited = Set<URL>()
    var target = url
    for _ in 0..<40 {
      let parent = target.deletingLastPathComponent()
      if let existing = nearestExistingDirectory(to: parent) {
        directories.insert(existing)
        // This source is already live if the containing directory disappears
        // or moves. While the path is missing it detects recreation, allowing
        // us to descend to a fresh containing-directory vnode again.
        directories.insert(existing.deletingLastPathComponent())
      }
      guard visited.insert(target).inserted,
            let destination = try? FileManager.default.destinationOfSymbolicLink(atPath: target.path)
      else { break }
      target = destination.hasPrefix("/")
        ? URL(fileURLWithPath: destination).standardizedFileURL
        : parent.appendingPathComponent(destination).standardizedFileURL
    }
    return directories
  }

  private func nearestExistingDirectory(to directory: URL) -> URL? {
    var candidate = directory
    while identity(at: candidate, directoryOnly: true) == nil {
      let parent = candidate.deletingLastPathComponent()
      guard parent != candidate else { return nil }
      candidate = parent
    }
    return candidate
  }

  /// Returns whether the path's identity changed. Watching the containing
  /// directory lets a missing path acquire a fresh vnode source when recreated.
  @discardableResult
  private func refreshFileWatch() -> Bool {
    let identity = identity(at: url)
    guard identity != watchedFileIdentity || (identity != nil && fileSource == nil) else {
      return false
    }
    fileSource?.cancel()
    fileSource = nil
    watchedFileIdentity = nil
    if identity != nil {
      fileSource = makeSource(
        at: url,
        events: [.write, .extend, .attrib, .link, .rename, .delete, .revoke],
        openedIdentity: { self.watchedFileIdentity = $0 }
      ) { [weak self] in
        guard let self, !self.isStopped else { return }
        self.refreshDirectoryWatches()
        self.refreshFileWatch()
        self.scheduleNotification()
      }
    }
    return true
  }

  private func identity(at url: URL, directoryOnly: Bool = false) -> FileIdentity? {
    var info = stat()
    let result = url.withUnsafeFileSystemRepresentation { path in
      path.map { fstatat(AT_FDCWD, $0, &info, 0) } ?? -1
    }
    guard result == 0, !directoryOnly || info.st_mode & S_IFMT == S_IFDIR else { return nil }
    return FileIdentity(info)
  }

  private func makeSource(
    at url: URL,
    events: DispatchSource.FileSystemEvent,
    openedIdentity: ((FileIdentity) -> Void)? = nil,
    onEvent: @escaping @MainActor () -> Void
  ) -> DispatchSourceFileSystemObject? {
    let descriptor = url.withUnsafeFileSystemRepresentation { path in
      path.map { Darwin.open($0, O_EVTONLY | O_CLOEXEC) } ?? -1
    }
    guard descriptor >= 0 else { return nil }
    var info = stat()
    guard fstat(descriptor, &info) == 0 else {
      Darwin.close(descriptor)
      return nil
    }
    openedIdentity?(FileIdentity(info))
    let source = DispatchSource.makeFileSystemObjectSource(
      fileDescriptor: descriptor, eventMask: events, queue: .main)
    source.setEventHandler {
      MainActor.assumeIsolated { onEvent() }
    }
    // A cancelled source can still have a handler in flight. Closing only in
    // its cancellation handler prevents descriptor reuse underneath dispatch.
    source.setCancelHandler { Darwin.close(descriptor) }
    source.resume()
    return source
  }

  private func scheduleNotification() {
    guard pendingNotification == nil else { return }
    let scheduledGeneration = generation
    let work = DispatchWorkItem { [weak self] in
      MainActor.assumeIsolated {
        guard let self, !self.isStopped, self.generation == scheduledGeneration else { return }
        self.pendingNotification = nil
        self.refreshDirectoryWatches()
        self.refreshFileWatch()
        self.onChange()
      }
    }
    pendingNotification = work
    // Bound the delay from the first event so continuous writes cannot postpone
    // notification indefinitely, while combining an atomic save's event burst.
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.075, execute: work)
  }
}
