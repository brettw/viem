import Foundation
import XCTest
@testable import ViemAppShell

final class EVFileFingerprintTests: XCTestCase {
  private func destination() throws -> URL {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent("viem-fingerprint-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return directory.appendingPathComponent("source.bin")
  }

  func testStreamingDigestMatchesBytesAcrossMultipleChunksAndEmptyFiles() throws {
    let url = try destination()
    let chunk = EVFileFingerprint.readChunkSize
    let payload = Data((0..<(2 * chunk + 317)).map { UInt8(truncatingIfNeeded: $0 &* 37) })
    for length in [0, 1, chunk - 1, chunk, chunk + 1, payload.count] {
      let data = Data(payload.prefix(length))
      try data.write(to: url)
      XCTAssertEqual(try EVFileFingerprint.read(url), EVFileFingerprint.bytes(data, at: url),
        "Length \(length) must hash exactly, including a final partial chunk")
    }
  }

  func testOpenedDescriptorKeepsIdentityWhenPathIsReplaced() throws {
    let url = try destination()
    let original = Data(repeating: 0x61, count: 2 * EVFileFingerprint.readChunkSize + 17)
    try original.write(to: url)
    let expected = EVFileFingerprint.bytes(original, at: url)
    let file = try FileHandle(forReadingFrom: url)
    defer { try? file.close() }
    let replacement = Data("replacement contents".utf8)
    try replacement.write(to: url, options: .atomic)

    XCTAssertEqual(try EVFileFingerprint.read(from: file), expected,
      "The old descriptor must not be labeled with the new path's inode")
    let current = try EVFileFingerprint.read(url)
    XCTAssertEqual(current, EVFileFingerprint.bytes(replacement, at: url))
    XCTAssertEqual(current.change(from: expected), .replaced)
  }

  func testDeletedMissingAndUnreadableDestinationsStayDistinct() throws {
    let url = try destination()
    let missing = try EVFileFingerprint.read(url)
    XCTAssertNil(missing.digest)
    XCTAssertNil(missing.device)
    XCTAssertNil(missing.inode)
    try Data("original".utf8).write(to: url)
    let original = try EVFileFingerprint.read(url)
    try FileManager.default.removeItem(at: url)
    XCTAssertEqual(try EVFileFingerprint.read(url), missing)
    XCTAssertEqual(try EVFileFingerprint.read(url).change(from: original), .deleted)
    // Directory read failure is deterministic even when tests run with elevated
    // privileges; a failed read must never masquerade as a missing file.
    XCTAssertThrowsError(try EVFileFingerprint.read(url.deletingLastPathComponent()))
  }
}
