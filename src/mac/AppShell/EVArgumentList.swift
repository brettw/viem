import Foundation

/// A parsed core command. Indexes supplied by the user are one-based; the
/// resolved list position used by the host is zero-based.
public struct EVArgumentNavigation: Equatable, Sendable {
  public enum Target: UInt32, Equatable, Sendable {
    case next = 1, previous = 2, first = 3, last = 4, index = 5, current = 6
  }

  public let target: Target
  public let count: UInt64
  public let writeFirst: Bool
  /// A one-based hard line; zero denotes the last line for a bare Ex `+`.
  public let line: UInt64?

  public init(target: Target, count: UInt64 = 1, writeFirst: Bool = false, line: UInt64? = nil) {
    self.target = target
    self.count = count
    self.writeFirst = writeFirst
    self.line = line
  }
}

public enum EVArgumentListError: LocalizedError, Equatable {
  case empty, beforeFirst, afterLast, invalidIndex

  public var errorDescription: String? {
    switch self {
    case .empty: "The argument list is empty."
    case .beforeFirst: "Cannot go before the first file."
    case .afterLast: "Cannot go beyond the last file."
    case .invalidIndex: "The argument number is outside the file list."
    }
  }
}

/// Installed by the core-backed editor composition. AppKit supplies file
/// identities and view state; the portable command module resolves navigation.
@MainActor public enum EVArgumentListPolicy {
  public static var resolve: ((Int, Int?, Int?, EVArgumentNavigation) -> Result<Int, Error>)?
}

/// URLs are captured at launch, so changing the working directory cannot
/// change what an argument names. Each pane retains its own last list position.
final class EVArgumentList {
  let urls: [URL]
  init(_ urls: [URL]) { self.urls = urls.map(EVDocumentIdentity.canonicalURL) }

  func index(of url: URL?, preferring previous: Int?) -> Int? {
    guard let url else { return nil }
    if let previous, urls.indices.contains(previous), EVDocumentIdentity.sameFile(url, urls[previous]) {
      return previous
    }
    return urls.firstIndex { EVDocumentIdentity.sameFile(url, $0) }
  }
}
