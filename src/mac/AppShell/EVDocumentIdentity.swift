import AppKit

/// File identity is shared by open, split, and recovery naming. Symlink aliases
/// use the target path; hard links additionally compare device/inode identity.
public enum EVDocumentIdentity {
  public static func canonicalURL(_ url: URL) -> URL {
    url.standardizedFileURL.resolvingSymlinksInPath().standardizedFileURL
  }

  public static func sameFile(_ first: URL, _ second: URL) -> Bool {
    let a = canonicalURL(first)
    let b = canonicalURL(second)
    if a == b { return true }
    guard let x = try? FileManager.default.attributesOfItem(atPath: a.path),
      let y = try? FileManager.default.attributesOfItem(atPath: b.path),
      let xd = x[.systemNumber] as? NSNumber, let yd = y[.systemNumber] as? NSNumber,
      let xi = x[.systemFileNumber] as? NSNumber, let yi = y[.systemFileNumber] as? NSNumber
    else { return false }
    return xd == yd && xi == yi
  }

  @MainActor public static func existingDocument(at url: URL) -> EVDocument? {
    NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }.first {
      $0.fileURL.map { sameFile($0, url) } ?? false
    }
  }

  @MainActor public static func open(
    _ url: URL, display: Bool,
    completion: @escaping @MainActor (EVDocument?, Error?) -> Void
  ) {
    if let existing = existingDocument(at: url) {
      existing.recordRecentDocument(canonicalURL(url))
      if display {
        if existing.windowControllers.isEmpty { existing.makeWindowControllers() }
        existing.showWindows()
      }
      completion(existing, nil)
      return
    }
    NSDocumentController.shared.openDocument(withContentsOf: canonicalURL(url), display: display) {
      document, _, error in
      completion(document as? EVDocument, error)
    }
  }
}
