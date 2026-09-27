import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVConfigurationStoreTests: XCTestCase {
  private func fixture() throws -> (URL, UserDefaults) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-json-\(UUID().uuidString)")
    let name = "viem-json-legacy-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
    addTeardownBlock { try? FileManager.default.removeItem(at: directory); defaults.removePersistentDomain(forName: name) }
    return (directory, defaults)
  }
  func testLegacyMigrationThenJSONIsTheOnlyAuthority() throws {
    let (directory, legacy) = try fixture()
    legacy.set(try JSONEncoder().encode(EVTheme.midnight), forKey: "EVApplicationTheme.v1")
    legacy.set(true, forKey: "EVEditing.SmartQuotes.v1")
    legacy.set(false, forKey: "EVShowStatusBar")
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertEqual(store.theme, .midnight); XCTAssertTrue(store.smartQuotes); XCTAssertFalse(store.showStatusBar)
    XCTAssertNil(legacy.object(forKey: "EVEditing.SmartQuotes.v1"))
    legacy.set(false, forKey: "EVEditing.SmartQuotes.v1")
    let reopened = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertTrue(reopened.smartQuotes)
    try reopened.setTheme(.paper)
    try reopened.setSmartQuotes(false)
    try reopened.setShowStatusBar(true)
    let final = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertEqual(final.theme, .paper); XCTAssertFalse(final.smartQuotes); XCTAssertTrue(final.showStatusBar)
  }
  func testUnknownKeysSurviveUpdatesAndIndependentStoresMerge() throws {
    let (directory, legacy) = try fixture()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let file = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"future":{"enabled":true},"editing":{"futureMode":"x","smartQuotes":false}}"#.utf8).write(to: file)
    let first = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    let second = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    try first.setTheme(.midnight); try second.setSmartQuotes(true)
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    XCTAssertEqual((object["future"] as? [String: Bool])?["enabled"], true)
    XCTAssertEqual((object["editing"] as? [String: Any])?["futureMode"] as? String, "x")
    let reopened = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertEqual(reopened.theme, .midnight); XCTAssertTrue(reopened.smartQuotes)
  }
  func testInvalidVersionValuesAndMalformedJSONAreNeverOverwritten() throws {
    for text in [#"{"version":2}"#, #"{"version":true}"#, #"{"version":1,"editing":{"smartQuotes":1}}"#, "{broken"] {
      let (directory, legacy) = try fixture()
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      let file = directory.appendingPathComponent("config.json")
      let before = Data(text.utf8); try before.write(to: file)
      let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
      XCTAssertNotNil(store.lastError); XCTAssertFalse(store.smartQuotes)
      XCTAssertThrowsError(try store.setSmartQuotes(true))
      XCTAssertEqual(try Data(contentsOf: file), before)
    }
  }
  func testWriteFailureDoesNotPublishSettingsChange() throws {
    let (directory, legacy) = try fixture()
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    try Data("obstruction".utf8).write(to: directory)
    let preferences = EVEditingPreferences(configuration: store)
    preferences.setSmartQuotes(true)
    XCTAssertFalse(preferences.smartQuotes); XCTAssertFalse(store.smartQuotes)
    XCTAssertEqual(try Data(contentsOf: directory), Data("obstruction".utf8))
  }
  func testInvalidExternalConfigurationChangeIsReportedWithoutPublishing() throws {
    let (directory, legacy) = try fixture()
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    try store.setSmartQuotes(true)
    let file = directory.appendingPathComponent("config.json")
    let external = Data(#"{"version":2,"future":true}"#.utf8)
    try external.write(to: file, options: .atomic)
    XCTAssertThrowsError(try store.setSmartQuotes(false))
    XCTAssertNotNil(store.lastError)
    XCTAssertTrue(store.smartQuotes)
    XCTAssertEqual(try Data(contentsOf: file), external)
  }
  func testTextWidthDefaultsToEightyPersistsAndPreservesUnrelatedSettings() throws {
    let (directory, legacy) = try fixture()
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertEqual(store.textWidth, 80)
    try store.setSmartQuotes(true)
    try store.setTextWidth(72)
    XCTAssertEqual(store.textWidth, 72)
    let reopened = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertEqual(reopened.textWidth, 72); XCTAssertTrue(reopened.smartQuotes)
    XCTAssertThrowsError(try reopened.setTextWidth(0))
    XCTAssertEqual(reopened.textWidth, 72)
    let file = directory.appendingPathComponent("config.json")
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    XCTAssertEqual((object["editing"] as? [String: Any])?["textWidth"] as? Int, 72)
    XCTAssertEqual((object["editing"] as? [String: Any])?["smartQuotes"] as? Bool, true)
  }
  func testInvalidTextWidthValuesAreRejectedWithoutOverwriting() throws {
    for value in ["0", "-1", "1.5", "true", "\"80\"", "4294967296", "null"] {
      let (directory, legacy) = try fixture()
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      let file = directory.appendingPathComponent("config.json")
      let before = Data(#"{"version":1,"editing":{"textWidth":\#(value)}}"#.utf8); try before.write(to: file)
      let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
      XCTAssertNotNil(store.lastError, value); XCTAssertEqual(store.textWidth, 80, value)
      XCTAssertThrowsError(try store.setTextWidth(72), value)
      XCTAssertEqual(try Data(contentsOf: file), before, value)
    }
    let (directory, legacy) = try fixture()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let file = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"editing":{"textWidth":4294967295}}"#.utf8).write(to: file)
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertNil(store.lastError); XCTAssertEqual(store.textWidth, UInt32.max)
  }
  func testStyleFilesAreFormatSpecificVersionedAndPreserveUnknownData() throws {
    let (directory, legacy) = try fixture()
    let store = EVConfigurationStore(directory: directory, legacyDefaults: legacy)
    XCTAssertNil(try store.styleDefaults(named: "text"))
    let original = Data(#"{"version":1,"future":42,"block_styles":[{"id":"Document","futureProperty":3,"character":{"futureFont":4}}],"character_styles":[]}"#.utf8)
    try store.saveStyleDefaults(original, named: "rtf")
    let update = Data(#"{"version":1,"block_styles":[{"id":"Document","character":{"size":19}}],"character_styles":[]}"#.utf8)
    try store.saveStyleDefaults(update, named: "rtf")
    let raw = try XCTUnwrap(store.styleDefaults(named: "rtf"))
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: raw) as? [String: Any])
    XCTAssertEqual(object["future"] as? Int, 42)
    let definition = try XCTUnwrap((object["block_styles"] as? [[String: Any]])?.first)
    XCTAssertEqual(definition["futureProperty"] as? Int, 3)
    XCTAssertEqual((definition["character"] as? [String: Int])?["futureFont"], 4)
    XCTAssertNil(try store.styleDefaults(named: "markdown"))
    XCTAssertThrowsError(try store.saveStyleDefaults(Data(#"{"version":2}"#.utf8), named: "rtf"))
    XCTAssertEqual(try store.styleDefaults(named: "rtf"), raw)
    XCTAssertThrowsError(try store.styleDefaults(named: "../config"))
  }
}
