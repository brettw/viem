import AppKit
import CryptoKit
import Darwin

public enum EVExternalFileChange: Equatable, Sendable {
  case modified, replaced, deleted, unreadable(String)
  public var message: String {
    switch self {
    case .modified: "The file changed outside Viem."
    case .replaced: "The file was replaced outside Viem."
    case .deleted: "The file was deleted outside Viem. Your buffer is unchanged. Save to choose whether to recreate it."
    case let .unreadable(reason): "The file could not be checked: \(reason). Your buffer is unchanged."
    }
  }
}

public enum EVExternalFileError: LocalizedError {
  case changed(EVExternalFileChange)
  public var errorDescription: String? { if case let .changed(change) = self { return change.message }; return nil }
}

struct EVFileFingerprint: Equatable, Sendable {
  let digest: Data?
  let device: UInt64?
  let inode: UInt64?
  static func bytes(_ data: Data, at url: URL) -> Self {
    let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
    return Self(digest: Data(SHA256.hash(data: data)),
      device: (attributes?[.systemNumber] as? NSNumber)?.uint64Value,
      inode: (attributes?[.systemFileNumber] as? NSNumber)?.uint64Value)
  }
  static let readChunkSize = 1024 * 1024

  static func read(_ url: URL) throws -> Self {
    guard url.isFileURL else { throw CocoaError(.fileReadUnsupportedScheme) }
    let (descriptor, openError) = try url.withUnsafeFileSystemRepresentation { path -> (Int32, Int32) in
      guard let path else { throw CocoaError(.fileReadInvalidFileName) }
      let descriptor = Darwin.open(path, O_RDONLY | O_CLOEXEC)
      return (descriptor, errno)
    }
    if descriptor < 0 {
      if openError == ENOENT { return Self(digest: nil, device: nil, inode: nil) }
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(openError),
        userInfo: [NSFilePathErrorKey: url.path])
    }
    let file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    defer { try? file.close() }
    return try read(from: file)
  }

  /// Consumes a newly opened descriptor using one reusable buffer. Descriptor
  /// metadata identifies the bytes actually hashed even if the pathname is
  /// replaced during the read. The caller retains ownership of the descriptor.
  static func read(from file: FileHandle) throws -> Self {
    let descriptor = file.fileDescriptor
    var metadata = stat()
    guard fstat(descriptor, &metadata) == 0 else {
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
    guard metadata.st_mode & S_IFMT != S_IFDIR else {
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(EISDIR))
    }
    var hash = SHA256()
    var buffer = [UInt8](repeating: 0, count: readChunkSize)
    try buffer.withUnsafeMutableBytes { bytes in
      while true {
        let count = Darwin.read(descriptor, bytes.baseAddress, bytes.count)
        if count == 0 { break }
        if count < 0 {
          let code = errno
          if code == EINTR { continue }
          throw NSError(domain: NSPOSIXErrorDomain, code: Int(code))
        }
        hash.update(bufferPointer: UnsafeRawBufferPointer(start: bytes.baseAddress, count: count))
      }
    }
    return Self(digest: Data(hash.finalize()),
      device: UInt64(truncatingIfNeeded: metadata.st_dev), inode: UInt64(metadata.st_ino))
  }
  func change(from previous: Self) -> EVExternalFileChange? {
    if digest == nil { return previous.digest == nil ? nil : .deleted }
    if device != previous.device || inode != previous.inode { return .replaced }
    if digest != previous.digest { return .modified }
    return nil
  }
}

struct EVExternalWriteAuthorization {
  let fingerprint: EVFileFingerprint
  let overwriteApproved: Bool
}

@MainActor
extension EVDocument {
  /// Baseline is a compact fingerprint of the exact source loaded or written,
  /// not another retained full-document byte copy.
  func recordFileBaseline(_ data: Data, at url: URL, format: EVSourceFormat? = nil,
                          fingerprint: EVFileFingerprint? = nil) {
    fileBaselineURL = EVDocumentIdentity.canonicalURL(url)
    fileBaselineFormat = format ?? editorBackend.sourceFormat
    fileBaseline = fingerprint ?? EVFileFingerprint.bytes(data, at: fileBaselineURL!)
    fileBaselineGeneration &+= 1
    externalFileChange = nil
    resetExternalFileReview()
    updateExternalFileMonitorForBaseline(at: url)
  }
  func recordMissingFileBaseline(at url: URL) {
    fileBaselineURL = EVDocumentIdentity.canonicalURL(url)
    fileBaselineFormat = editorBackend.sourceFormat
    fileBaseline = EVFileFingerprint(digest: nil, device: nil, inode: nil)
    fileBaselineGeneration &+= 1
    externalFileChange = nil
    resetExternalFileReview()
    updateExternalFileMonitorForBaseline(at: url)
  }

  /// Explicit checks and activation do I/O off the document actor. A stale
  /// result after reload/save/retarget is ignored rather than warning about an
  /// obsolete file. This never reloads or changes the editing buffer.
  public func checkForExternalChanges(completion: @escaping @MainActor (EVExternalFileChange?) -> Void) {
    guard !externalFileMonitoringClosed else { completion(nil); return }
    externalFileCheckCompletions.append(completion)
    if !externalFileCheckInFlight { performExternalFileCheck() }
  }

  private func performExternalFileCheck() {
    let completions = externalFileCheckCompletions
    externalFileCheckCompletions.removeAll()
    guard !externalFileMonitoringClosed, let baseline = fileBaseline,
          let target = fileURL ?? fileBaselineURL else {
      completions.forEach { $0(nil) }; return
    }
    let generation = fileBaselineGeneration
    externalFileCheckInFlight = true
    DispatchQueue.global(qos: .utility).async { [weak self] in
      let observation: EVExternalFileObservation?
      do {
        let fingerprint = try EVFileFingerprint.read(target)
        observation = fingerprint.change(from: baseline).map {
          EVExternalFileObservation(change: $0, fingerprint: fingerprint)
        }
      } catch {
        observation = EVExternalFileObservation(change: .unreadable(error.localizedDescription), fingerprint: nil)
      }
      DispatchQueue.main.async {
        guard let self else { completions.forEach { $0(nil) }; return }
        self.externalFileCheckInFlight = false
        if !self.externalFileMonitoringClosed, self.fileBaselineGeneration == generation {
          self.externalFileObservation = observation
          self.externalFileChange = observation?.change
          if observation == nil { self.externalFileReviewState.clearAcknowledged() }
          completions.forEach { $0(observation?.change) }
        } else { completions.forEach { $0(nil) } }
        // At most one read is active; a burst while it runs shares one trailing
        // check instead of spawning unbounded whole-file hash jobs.
        if !self.externalFileCheckInFlight, !self.externalFileCheckCompletions.isEmpty {
          self.performExternalFileCheck()
        }
      }
    }
  }

  /// Every explicit write checks the destination, including forced Ex writes.
  /// A force flag controls read-only/existing-destination policy; it never
  /// silently authorizes loss of changes made after the document was loaded.
  /// Returning the accepted state also lets the actual writer detect a change
  /// while its immutable snapshot was queued or its temporary file was built.
  func authorizeExternalWrite(to url: URL) throws -> EVExternalWriteAuthorization {
    // Writers validate the requested pathname before resolving it. This layer
    // also receives canonical URLs from AppKit and background write retries.
    try validatePreservedOriginal(at: url, checkDestinationName: false)
    let observed: EVFileFingerprint
    do { observed = try EVFileFingerprint.read(url) }
    catch {
      externalFileChange = .unreadable(error.localizedDescription)
      // An unreadable destination cannot be safely verified or replaced.
      throw EVExternalFileError.changed(externalFileChange!)
    }
    let baseline: EVFileFingerprint?
    if let current = fileURL ?? fileBaselineURL,
       current.standardizedFileURL == url.standardizedFileURL || EVDocumentIdentity.sameFile(current, url) {
      baseline = fileBaseline
    } else {
      // A Save As destination may already be open in a different buffer.
      baseline = EVDocumentIdentity.existingDocument(at: url)?.fileBaseline
    }
    let change = baseline.flatMap { observed.change(from: $0) }
    externalFileChange = change
    if let change {
      guard confirmExternalOverwrite(change) else { throw CocoaError(.userCancelled) }
      return EVExternalWriteAuthorization(fingerprint: observed, overwriteApproved: true)
    }
    return EVExternalWriteAuthorization(fingerprint: observed, overwriteApproved: false)
  }

  /// A late change is a different disk state, so it needs a fresh choice.
  /// The state already accepted by Save Anyway does not prompt a second time.
  func authorizeExternalWrite(to url: URL, since expected: EVFileFingerprint) throws -> EVExternalWriteAuthorization {
    try validatePreservedOriginal(at: url, checkDestinationName: false)
    let observed = try EVFileFingerprint.read(url)
    if let change = observed.change(from: expected) {
      externalFileChange = change
      guard confirmExternalOverwrite(change) else { throw CocoaError(.userCancelled) }
      return EVExternalWriteAuthorization(fingerprint: observed, overwriteApproved: true)
    }
    return EVExternalWriteAuthorization(fingerprint: observed, overwriteApproved: false)
  }

  func confirmExternalOverwrite(_ change: EVExternalFileChange) -> Bool {
    if let handler = externalSaveDecisionHandler { return handler(change) }
    let alert = NSAlert()
    alert.alertStyle = .critical
    alert.messageText = "The file changed outside Viem."
    let explanation: String
    switch change {
    case .modified: explanation = "The file has been modified since Viem last read or saved it."
    case .replaced: explanation = "The file has been replaced since Viem last read or saved it."
    case .deleted: explanation = "The file has been deleted since Viem last read or saved it."
    case let .unreadable(reason): explanation = "The file cannot be checked: \(reason)."
    }
    alert.informativeText = "\(explanation) Save Anyway replaces the file on disk with this buffer’s contents, recreating it if necessary. Cancel keeps both versions unchanged."
    alert.addButton(withTitle: "Cancel")
    alert.addButton(withTitle: "Save Anyway")
    return alert.runModal() == .alertSecondButtonReturn
  }
}
