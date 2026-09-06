import Foundation
import Darwin

/// Writes an immutable source snapshot. Exclusive linking closes the race
/// between checking an alternate filename and publishing the finished bytes.
enum EVExFileWriter {
  static func write(_ data: Data, to destination: URL, force: Bool, expected: EVFileFingerprint? = nil) throws {
    let target = destination.standardizedFileURL
    let temporary = target.deletingLastPathComponent().appendingPathComponent(".evim-write-\(UUID().uuidString)")
    try data.write(to: temporary, options: .withoutOverwriting)
    defer { try? FileManager.default.removeItem(at: temporary) }
    if let permissions = try? FileManager.default.attributesOfItem(atPath: target.path)[.posixPermissions] {
      try FileManager.default.setAttributes([.posixPermissions: permissions], ofItemAtPath: temporary.path)
    }
    let file = try FileHandle(forWritingTo: temporary)
    try file.synchronize()
    try file.close()
    if let expected, let change = try EVFileFingerprint.read(target).change(from: expected) {
      throw EVExternalFileError.changed(change)
    }
    let result = temporary.path.withCString { from in
      target.path.withCString { to in force ? rename(from, to) : link(from, to) }
    }
    guard result == 0 else {
      if errno == EEXIST { throw EVDocumentHostError.destinationExists(target.path) }
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno), userInfo: [NSFilePathErrorKey: target.path])
    }
  }
}
