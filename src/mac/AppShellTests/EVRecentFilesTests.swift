import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVRecentFilesTests: XCTestCase {
  private func fixture() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-recent-files-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory, legacyDefaults: nil)
  }

  private func file(_ configuration: EVConfigurationStore, _ name: String) -> URL {
    configuration.directory.appendingPathComponent("documents").appendingPathComponent(name)
  }

  private func canonical(_ url: URL) -> URL { EVDocumentIdentity.canonicalURL(url) }

  func testMostRecentTenFullPathsPersistAndOpeningAnExistingEntryPromotesIt() throws {
    let configuration = fixture()
    XCTAssertTrue(configuration.recentDocumentURLs.isEmpty)
    let files = (0..<12).map { file(configuration, "draft \($0) naïve.md") }
    for url in files { try configuration.recordRecentDocument(url) }
    var expected = Array(files.reversed().prefix(10)).map(canonical)
    XCTAssertEqual(configuration.recentDocumentURLs, expected)
    let reopened = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
    XCTAssertEqual(reopened.recentDocumentURLs, expected)
    try reopened.recordRecentDocument(files[6])
    expected.removeAll { $0 == canonical(files[6]) }
    expected.insert(canonical(files[6]), at: 0)
    XCTAssertEqual(reopened.recentDocumentURLs, expected)
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf:
      configuration.directory.appendingPathComponent("config.json"))) as? [String: Any])
    XCTAssertEqual(object["recentDocuments"] as? [String], expected.map(\.path))
    XCTAssertEqual(expected.count, 10)
    XCTAssertTrue(expected.allSatisfy { $0.isFileURL && $0.path.hasPrefix("/") })
  }

  func testSymlinksAndHardLinksReferToOneRecentFileButMatchingNamesDoNot() throws {
    let configuration = fixture()
    let original = file(configuration, "original.txt")
    let symbolic = file(configuration, "symbolic.txt")
    let hard = file(configuration, "hard.txt")
    let other = file(configuration, "other/original.txt")
    try FileManager.default.createDirectory(at: other.deletingLastPathComponent(), withIntermediateDirectories: true)
    try Data("original".utf8).write(to: original)
    try Data("different".utf8).write(to: other)
    try FileManager.default.createSymbolicLink(at: symbolic, withDestinationURL: original)
    try FileManager.default.linkItem(at: original, to: hard)
    try configuration.recordRecentDocument(original)
    try configuration.recordRecentDocument(other)
    try configuration.recordRecentDocument(symbolic)
    XCTAssertEqual(configuration.recentDocumentURLs, [canonical(original), canonical(other)])
    try configuration.recordRecentDocument(hard)
    XCTAssertEqual(configuration.recentDocumentURLs, [canonical(hard), canonical(other)])
    let reopened = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
    XCTAssertEqual(reopened.recentDocumentURLs.count, 2)
    try reopened.recordRecentDocument(original)
    XCTAssertEqual(reopened.recentDocumentURLs, [canonical(original), canonical(other)])
  }

  func testIndependentStoresMergeFreshOrderClearPersistentlyAndKeepUnknownSettings() throws {
    let configuration = fixture()
    try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
    let settings = configuration.directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"future":{"enabled":true},"appearance":{"futureAccent":42}}"#.utf8).write(to: settings)
    let first = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
    let second = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
    let files = (0..<4).map { file(configuration, "\($0).txt") }
    try first.recordRecentDocument(files[0])
    try second.recordRecentDocument(files[1])
    try first.recordRecentDocument(files[2])
    XCTAssertEqual(first.recentDocumentURLs, [files[2], files[1], files[0]].map(canonical))
    try second.clearRecentDocuments()
    XCTAssertTrue(EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil).recentDocumentURLs.isEmpty)
    try first.recordRecentDocument(files[3])
    XCTAssertEqual(first.recentDocumentURLs, [canonical(files[3])], "A stale store must not resurrect cleared entries")
    try second.setShowStatusBar(false)
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: settings)) as? [String: Any])
    XCTAssertEqual(object["recentDocuments"] as? [String], [canonical(files[3]).path])
    XCTAssertEqual((object["future"] as? [String: Bool])?["enabled"], true)
    XCTAssertEqual((object["appearance"] as? [String: Any])?["futureAccent"] as? Int, 42)
    XCTAssertEqual((object["appearance"] as? [String: Any])?["showStatusBar"] as? Bool, false)
  }

  func testRecordingTheFirstFileRefreshesStaleMemoryWithoutRewritingConfig() throws {
    let configuration = fixture()
    let first = file(configuration, "first.txt")
    let second = file(configuration, "second.txt")
    try configuration.recordRecentDocument(first)
    let stale = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
    try configuration.recordRecentDocument(second)
    let settings = configuration.directory.appendingPathComponent("config.json")
    // Keep distinct formatting and an old modification date: a redundant
    // atomic write would replace both even if its JSON values were equal.
    let object = try JSONSerialization.jsonObject(with: Data(contentsOf: settings))
    let compact = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    try compact.write(to: settings)
    let stamp = Date(timeIntervalSince1970: 123_456)
    try FileManager.default.setAttributes([.modificationDate: stamp], ofItemAtPath: settings.path)
    try stale.recordRecentDocument(second)
    XCTAssertEqual(stale.recentDocumentURLs, [canonical(second), canonical(first)])
    XCTAssertEqual(try Data(contentsOf: settings), compact)
    XCTAssertEqual(try FileManager.default.attributesOfItem(atPath: settings.path)[.modificationDate] as? Date, stamp)
    XCTAssertNil(stale.lastError)

    try FileManager.default.removeItem(at: settings)
    try stale.recordRecentDocument(second)
    XCTAssertEqual(EVConfigurationStore(directory: configuration.directory).recentDocumentURLs,
      [canonical(second), canonical(first)], "A missing settings file must be recreated even when recency is unchanged")
  }

  func testFailedWritesKeepPublishedRecentFilesAndReportTheFailure() throws {
    let configuration = fixture()
    let first = file(configuration, "first.txt")
    try configuration.recordRecentDocument(first)
    try FileManager.default.removeItem(at: configuration.directory)
    let obstruction = Data("not a directory".utf8)
    try obstruction.write(to: configuration.directory)
    XCTAssertThrowsError(try configuration.recordRecentDocument(file(configuration, "second.txt")))
    XCTAssertEqual(configuration.recentDocumentURLs, [canonical(first)])
    XCTAssertNotNil(configuration.lastError)
    XCTAssertThrowsError(try configuration.clearRecentDocuments())
    XCTAssertEqual(configuration.recentDocumentURLs, [canonical(first)])
    XCTAssertEqual(try Data(contentsOf: configuration.directory), obstruction)
  }

  func testMalformedRecentFileSettingsAndInvalidPathsAreNeverWrittenOver() throws {
    let invalidValues: [Any] = [false, [1], ["relative.txt"], [""], ["/a\0b"],
      ["/" + String(repeating: "é", count: 8192)], Array(repeating: "/file.txt", count: 11)]
    for value in invalidValues {
      let configuration = fixture()
      try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
      let settings = configuration.directory.appendingPathComponent("config.json")
      let invalid = try JSONSerialization.data(withJSONObject: ["version": 1, "recentDocuments": value])
      try invalid.write(to: settings)
      let reopened = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
      XCTAssertNotNil(reopened.lastError)
      XCTAssertTrue(reopened.recentDocumentURLs.isEmpty)
      XCTAssertThrowsError(try reopened.recordRecentDocument(file(configuration, "valid.txt")))
      XCTAssertThrowsError(try reopened.clearRecentDocuments())
      XCTAssertEqual(try Data(contentsOf: settings), invalid)
    }
    let configuration = fixture()
    try configuration.recordRecentDocument(file(configuration, "valid.txt"))
    let before = configuration.recentDocumentURLs
    XCTAssertThrowsError(try configuration.recordRecentDocument(try XCTUnwrap(URL(string: "https://example.com/file.txt"))))
    XCTAssertThrowsError(try configuration.recordRecentDocument(URL(fileURLWithPath: "/" + String(repeating: "x", count: 16_384))))
    XCTAssertEqual(configuration.recentDocumentURLs, before)
  }
}
