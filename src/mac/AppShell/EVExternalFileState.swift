import AppKit
import CryptoKit

public enum EVExternalFileChange: Equatable, Sendable {
  case modified, replaced, deleted, unreadable(String)
  public var message: String {
    switch self {
    case .modified: "The file changed outside eVim. Your buffer is unchanged. Reload with :e! or use :w! to overwrite."
    case .replaced: "The file was replaced outside eVim. Your buffer is unchanged. Reload with :e! or use :w! to overwrite."
    case .deleted: "The file was deleted outside eVim. Your buffer is unchanged. Use :w! to recreate it."
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
  static func read(_ url: URL) throws -> Self {
    do { return bytes(try Data(contentsOf: url, options: .mappedIfSafe), at: url) }
    catch let error as CocoaError where error.code == .fileReadNoSuchFile { return Self(digest: nil, device: nil, inode: nil) }
  }
  func change(from previous: Self) -> EVExternalFileChange? {
    if digest == nil { return previous.digest == nil ? nil : .deleted }
    if device != previous.device || inode != previous.inode { return .replaced }
    if digest != previous.digest { return .modified }
    return nil
  }
}

@MainActor
extension EVDocument {
  /// Baseline is a compact fingerprint of the exact source loaded or written,
  /// not another retained full-document byte copy.
  func recordFileBaseline(_ data: Data, at url: URL) {
    fileBaselineURL = EVDocumentIdentity.canonicalURL(url)
    fileBaseline = EVFileFingerprint.bytes(data, at: fileBaselineURL!)
    fileBaselineGeneration &+= 1
    externalFileChange = nil
  }
  func recordMissingFileBaseline(at url: URL) {
    fileBaselineURL = EVDocumentIdentity.canonicalURL(url)
    fileBaseline = EVFileFingerprint(digest: nil, device: nil, inode: nil)
    fileBaselineGeneration &+= 1
    externalFileChange = nil
  }

  /// Explicit checks and activation do I/O off the document actor. A stale
  /// result after reload/save/retarget is ignored rather than warning about an
  /// obsolete file. This never reloads or changes the editing buffer.
  public func checkForExternalChanges(completion: @escaping @MainActor (EVExternalFileChange?) -> Void) {
    guard let baseline = fileBaseline, let target = fileURL ?? fileBaselineURL else { completion(nil); return }
    let generation = fileBaselineGeneration
    DispatchQueue.global(qos: .utility).async { [weak self] in
      let change: EVExternalFileChange?
      do { change = try EVFileFingerprint.read(target).change(from: baseline) }
      catch { change = .unreadable(error.localizedDescription) }
      DispatchQueue.main.async {
        guard let self, self.fileBaselineGeneration == generation else { completion(nil); return }
        self.externalFileChange = change
        completion(change)
      }
    }
  }

  /// Host preflight is synchronous only for the explicit write operation.
  /// Alternate new destinations use their own overwrite policy; this guard
  /// protects a file whose prior contents are known to the buffer.
  func validateExternalWrite(to url: URL, force: Bool) throws {
    guard !force, let baseline = fileBaseline, let current = fileURL ?? fileBaselineURL,
          current.standardizedFileURL == url.standardizedFileURL || EVDocumentIdentity.sameFile(current, url)
    else { return }
    let change: EVExternalFileChange?
    do { change = try EVFileFingerprint.read(url).change(from: baseline) }
    catch { change = .unreadable(error.localizedDescription) }
    externalFileChange = change
    if let change { throw EVExternalFileError.changed(change) }
  }

  func confirmExternalOverwrite(_ change: EVExternalFileChange) -> Bool {
    if let handler = externalSaveDecisionHandler { return handler(change) }
    let alert = NSAlert()
    alert.messageText = "The file changed outside eVim."
    alert.informativeText = "Saving will replace the current file with this buffer’s contents."
    alert.addButton(withTitle: "Overwrite")
    alert.addButton(withTitle: "Cancel")
    return alert.runModal() == .alertFirstButtonReturn
  }
}
