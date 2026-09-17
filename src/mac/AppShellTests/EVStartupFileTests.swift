import Foundation
import Darwin
import XCTest
@testable import ViemAppShell

@MainActor
final class EVStartupFileTests: XCTestCase {
  private func profile(create: Bool = true) throws -> URL {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-startup-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    if create { try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true) }
    return directory
  }

  func testMissingStartupFileIsEmptyAndDoesNotCreateTheProfile() throws {
    let directory = try profile(create: false)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.url, directory.appendingPathComponent("startup.viem"))
    XCTAssertEqual(startup.text, "")
    XCTAssertEqual(startup.diagnostics, [])
    XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path))
  }

  func testUTF8BOMIsRemovedAndAllOtherContentsRemainUnchanged() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    let text = "\" Écrivain settings\r\n\r\nmap Y y$\r\nmap <C-F2> :sp\r\n"
    let original = Data([0xEF, 0xBB, 0xBF]) + Data(text.utf8)
    try original.write(to: file)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.text, text)
    XCTAssertEqual(startup.diagnostics, [])
    XCTAssertEqual(try Data(contentsOf: file), original)
  }

  func testInvalidUTF8ReportsTheFilePathWithoutReturningPartialCommands() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    try (Data("map Y y$\n".utf8) + Data([0xC3, 0x28])).write(to: file)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.text, "")
    XCTAssertEqual(startup.diagnostics.count, 1)
    XCTAssertTrue(startup.diagnostics[0].contains(file.path))
    XCTAssertTrue(startup.diagnostics[0].contains("UTF-8"))
  }

  func testUnreadableStartupPathReportsTheFilePath() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    try FileManager.default.createDirectory(at: file, withIntermediateDirectories: false)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.text, "")
    XCTAssertEqual(startup.diagnostics.count, 1)
    XCTAssertTrue(startup.diagnostics[0].contains(file.path))
    XCTAssertTrue(startup.diagnostics[0].contains("Unable to read"))
  }

  func testFIFOStartupPathIsRejectedWithoutWaitingForAWriter() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    XCTAssertEqual(mkfifo(file.path, 0o600), 0)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.text, "")
    XCTAssertEqual(startup.diagnostics.count, 1)
    XCTAssertTrue(startup.diagnostics[0].contains(file.path))
    XCTAssertTrue(startup.diagnostics[0].contains("regular file"))
  }

  func testSymlinkToRegularStartupFileRemainsSupported() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    let target = directory.appendingPathComponent("shared-startup.viem")
    try Data("map Y y$\n".utf8).write(to: target)
    try FileManager.default.createSymbolicLink(at: file, withDestinationURL: target)
    let startup = EVStartupFile.load(directory: directory)
    XCTAssertEqual(startup.text, "map Y y$\n")
    XCTAssertEqual(startup.diagnostics, [])
    XCTAssertEqual(startup.url, file)
  }

  func testSizeLimitAcceptsOneMiBAndRejectsAnAdditionalByte() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    var contents = Data(repeating: 0x20, count: EVStartupFile.maximumBytes)
    try contents.write(to: file)
    let allowed = EVStartupFile.load(directory: directory)
    XCTAssertEqual(allowed.text.utf8.count, EVStartupFile.maximumBytes)
    XCTAssertEqual(allowed.diagnostics, [])

    contents.append(0x20)
    try contents.write(to: file)
    let oversized = EVStartupFile.load(directory: directory)
    XCTAssertEqual(oversized.text, "")
    XCTAssertEqual(oversized.diagnostics.count, 1)
    XCTAssertTrue(oversized.diagnostics[0].contains(file.path))
    XCTAssertTrue(oversized.diagnostics[0].contains("1 MiB"))
  }

  func testConfigurationSharesOneStartupSnapshotUntilReopened() throws {
    let directory = try profile()
    let file = directory.appendingPathComponent("startup.viem")
    let configuration = EVConfigurationStore(directory: directory)
    try Data("map Y y$\n".utf8).write(to: file)
    let first = configuration.startupFile
    XCTAssertEqual(first.text, "map Y y$\n")

    try Data("map Y yy\n".utf8).write(to: file)
    try configuration.setSmartQuotes(true)
    try configuration.reloadFromDisk()
    XCTAssertEqual(configuration.startupFile, first)
    XCTAssertEqual(EVConfigurationStore(directory: directory).startupFile.text, "map Y yy\n")
  }

  func testMissingStartupSnapshotRemainsEmptyWhenFileIsCreatedLater() throws {
    let directory = try profile()
    let configuration = EVConfigurationStore(directory: directory)
    XCTAssertEqual(configuration.startupFile.text, "")
    try Data("map Y y$\n".utf8).write(to: directory.appendingPathComponent("startup.viem"))
    XCTAssertEqual(configuration.startupFile.text, "")
    XCTAssertEqual(configuration.startupFile.diagnostics, [])
    XCTAssertEqual(EVConfigurationStore(directory: directory).startupFile.text, "map Y y$\n")
  }
}
