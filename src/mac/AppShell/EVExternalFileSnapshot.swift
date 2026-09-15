import CryptoKit
import Darwin
import Foundation

/// The exact candidate for a user-authorized reload. Both identity and digest
/// come from the descriptor that supplied these bytes, never a later path read.
struct EVExternalFileSnapshot: Sendable {
  let data: Data
  let fingerprint: EVFileFingerprint
  let modificationDate: Date

  static func read(_ url: URL) throws -> Self {
    let file = try FileHandle(forReadingFrom: url)
    defer { try? file.close() }
    var metadata = stat()
    guard fstat(file.fileDescriptor, &metadata) == 0 else {
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
    guard metadata.st_mode & S_IFMT != S_IFDIR else { throw CocoaError(.fileReadUnsupportedScheme) }
    let data = try file.readToEnd() ?? Data()
    return Self(data: data,
      fingerprint: EVFileFingerprint(digest: Data(SHA256.hash(data: data)),
        device: UInt64(truncatingIfNeeded: metadata.st_dev), inode: UInt64(metadata.st_ino)),
      modificationDate: Date(timeIntervalSince1970: TimeInterval(metadata.st_mtimespec.tv_sec)
        + TimeInterval(metadata.st_mtimespec.tv_nsec) / 1_000_000_000))
  }
}
