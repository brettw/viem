import Foundation
import XCTest
@testable import ViemAppShell

final class EVExFileWriterTests: XCTestCase {
  func testExclusiveWriteAndLateExternalChangeKeepPriorBytes() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-writer-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let destination = directory.appendingPathComponent("file.txt")
    try EVExFileWriter.write(Data("original".utf8), to: destination, force: false)
    let expected = try EVFileFingerprint.read(destination)
    XCTAssertThrowsError(try EVExFileWriter.write(Data("wrong".utf8), to: destination, force: false))
    XCTAssertEqual(try Data(contentsOf: destination), Data("original".utf8))
    try Data("external".utf8).write(to: destination)
    XCTAssertThrowsError(try EVExFileWriter.write(Data("queued snapshot".utf8), to: destination, force: true, expected: expected))
    XCTAssertEqual(try Data(contentsOf: destination), Data("external".utf8))
    XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["file.txt"])
    try EVExFileWriter.write(Data("forced".utf8), to: destination, force: true)
    XCTAssertEqual(try Data(contentsOf: destination), Data("forced".utf8))
  }
}
