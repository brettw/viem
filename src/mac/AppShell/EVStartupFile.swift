import Foundation
import Darwin

/// One read-only snapshot of the profile's startup commands. The portable core
/// interprets the text; the frontend only performs bounded file I/O and decoding.
public struct EVStartupFile: Equatable, Sendable {
  public let url: URL
  public let text: String
  public let diagnostics: [String]

  static let maximumBytes = 1024 * 1024

  public static func load(directory: URL) -> Self {
    let url = directory.appendingPathComponent("startup.viem")
    do {
      // A profile path may be a symlink, FIFO, or device. Open nonblocking and
      // validate the opened descriptor, avoiding both FIFO startup hangs and a
      // path-stat/open race while still allowing symlinks to ordinary files.
      var descriptor = Darwin.open(url.path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
      if descriptor < 0 && errno == ENOENT {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // Exclusive creation preserves a file installed by another process.
        descriptor = Darwin.open(url.path, O_RDONLY | O_CREAT | O_EXCL | O_NONBLOCK | O_CLOEXEC, 0o666)
        if descriptor < 0 && errno == EEXIST {
          descriptor = Darwin.open(url.path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
        }
      }
      guard descriptor >= 0 else {
        let code = errno
        throw NSError(domain: NSPOSIXErrorDomain, code: Int(code))
      }
      let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
      defer { try? handle.close() }
      var metadata = stat()
      guard fstat(descriptor, &metadata) == 0 else {
        throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
      }
      guard metadata.st_mode & S_IFMT == S_IFREG else {
        return Self(url: url, text: "", diagnostics: ["\(url.path): Unable to read startup file: expected a regular file."])
      }
      var data = Data()
      // Read at most one byte beyond the limit, even if the file grows while
      // being read. Metadata alone cannot enforce a bounded allocation.
      while data.count <= maximumBytes {
        let chunk = try handle.read(upToCount: min(64 * 1024, maximumBytes + 1 - data.count)) ?? Data()
        if chunk.isEmpty { break }
        data.append(chunk)
      }
      guard data.count <= maximumBytes else {
        return Self(url: url, text: "", diagnostics: ["\(url.path): Startup file exceeds 1 MiB."])
      }
      if data.starts(with: [0xEF, 0xBB, 0xBF]) { data.removeFirst(3) }
      guard let text = String(data: data, encoding: .utf8) else {
        return Self(url: url, text: "", diagnostics: ["\(url.path): Startup file must contain valid UTF-8."])
      }
      return Self(url: url, text: text, diagnostics: [])
    } catch {
      return Self(url: url, text: "", diagnostics: ["\(url.path): Unable to read startup file: \(error.localizedDescription)"])
    }
  }
}
