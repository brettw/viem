import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVCodePreferencesTests: XCTestCase {
  private func fixture() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory)
  }

  func testDirectoryDefaultsPersistenceUnknownKeysAndInvalidConfiguration() throws {
    let configuration = fixture()
    XCTAssertEqual(configuration.vimSyntaxDirectory, EVCodePreferences.defaultVimSyntaxDirectory)
    try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
    let file = configuration.directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"code":{"future":42}}"#.utf8).write(to: file)
    let preferences = EVCodePreferences(configuration: configuration)
    XCTAssertTrue(preferences.setVimSyntaxDirectory("/tmp/custom syntax"))
    let reopened = EVConfigurationStore(directory: configuration.directory)
    XCTAssertEqual(reopened.vimSyntaxDirectory, "/tmp/custom syntax")
    let stored = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    XCTAssertEqual((stored["code"] as? [String: Any])?["future"] as? Int, 42)
    XCTAssertNotNil(preferences.directoryDiagnostic)
    preferences.restoreDefaultDirectory()
    XCTAssertEqual(preferences.vimSyntaxDirectory, EVCodePreferences.defaultVimSyntaxDirectory)
    let invalid = Data(#"{"version":1,"code":{"vimSyntaxDirectory":false}}"#.utf8)
    try invalid.write(to: file)
    let bad = EVConfigurationStore(directory: configuration.directory)
    XCTAssertNotNil(bad.lastError)
    XCTAssertThrowsError(try bad.setVimSyntaxDirectory("/tmp"))
    XCTAssertEqual(try Data(contentsOf: file), invalid)
  }

  func testWriteFailureDoesNotPublishDirectoryChange() throws {
    let configuration = fixture()
    try Data("obstruction".utf8).write(to: configuration.directory)
    let preferences = EVCodePreferences(configuration: configuration)
    XCTAssertFalse(preferences.setVimSyntaxDirectory("/tmp/syntax"))
    XCTAssertEqual(preferences.vimSyntaxDirectory, EVCodePreferences.defaultVimSyntaxDirectory)
    XCTAssertNotNil(preferences.lastError)
  }

  func testFilenameAssociationsPersistInOrderAndRejectInvalidUpdatesAtomically() throws {
    let configuration = fixture()
    let entries = [EVCodeFilenameAssociation(pattern: "Buildfile", language: "rust"),
      EVCodeFilenameAssociation(pattern: "*.custom", language: "python")]
    try configuration.setCodeFilenameAssociations(entries)
    let file = configuration.directory.appendingPathComponent("config.json")
    let original = try Data(contentsOf: file)
    XCTAssertEqual(EVConfigurationStore(directory: configuration.directory).codeFilenameAssociations, entries)
    XCTAssertEqual(try JSONDecoder().decode([EVCodeFilenameAssociation].self, from: configuration.codeFilenameAssociationsJSON()), entries)
    for invalid in [
      [EVCodeFilenameAssociation(pattern: "", language: "rust")],
      [EVCodeFilenameAssociation(pattern: String(repeating: "é", count: 129), language: "rust")],
      [EVCodeFilenameAssociation(pattern: "a\0b", language: "rust")],
      [EVCodeFilenameAssociation(pattern: "*", language: String(repeating: "x", count: 129))],
      [EVCodeFilenameAssociation(pattern: "*", language: "rust;execute")],
      Array(repeating: entries[0], count: 257),
      Array(repeating: EVCodeFilenameAssociation(pattern: String(repeating: "\u{1}", count: 256), language: "rust"), count: 256),
    ] {
      XCTAssertThrowsError(try configuration.setCodeFilenameAssociations(invalid))
      XCTAssertEqual(configuration.codeFilenameAssociations, entries)
      XCTAssertEqual(try Data(contentsOf: file), original)
    }
    XCTAssertThrowsError(try configuration.setVimSyntaxDirectory(String(repeating: "x", count: 16_385)))
    XCTAssertEqual(try Data(contentsOf: file), original)
    let malformed = Data(#"{"version":1,"code":{"filenameAssociations":[{"pattern":"*","language":false}]}}"#.utf8)
    try malformed.write(to: file)
    let rejected = EVConfigurationStore(directory: configuration.directory)
    XCTAssertNotNil(rejected.lastError)
    XCTAssertEqual(rejected.codeFilenameAssociations, [])
    XCTAssertThrowsError(try rejected.setCodeFilenameAssociations(entries))
    XCTAssertEqual(try Data(contentsOf: file), malformed)
  }

  func testCodeStyleAuthorityDoesNotResurrectDeletedDeclarations() throws {
    let configuration = fixture()
    let old = Data(#"{"version":1,"block_styles":[{"id":"Document","character":{"size":19}}],"character_styles":[{"id":"keyword"}]}"#.utf8)
    let cleared = Data(#"{"version":1,"block_styles":[{"id":"Document","character":{}}],"character_styles":[],"suppressed":["keyword"]}"#.utf8)
    try configuration.saveCodeStyleSheet(old)
    try configuration.saveCodeStyleSheet(cleared)
    let stored = try XCTUnwrap(configuration.codeStyleSheet())
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: stored) as? [String: Any])
    let blocks = try XCTUnwrap(object["block_styles"] as? [[String: Any]])
    XCTAssertNil((blocks[0]["character"] as? [String: Any])?["size"])
    XCTAssertEqual((object["character_styles"] as? [[String: Any]])?.count, 0)
    XCTAssertNil(try configuration.styleDefaults(named: "text"))
  }

  func testCodeSettingsPageEditsDirectoryAndReportsUnavailableResources() throws {
    let configuration = fixture()
    let preferences = EVCodePreferences(configuration: configuration)
    let window = EVSettingsWindowController(store: EVThemeStore(configuration: configuration),
      editingPreferences: EVEditingPreferences(configuration: configuration), codePreferences: preferences)
    window.showCodeCategoryForTesting()
    XCTAssertEqual(window.codeDirectoryForTesting, EVCodePreferences.defaultVimSyntaxDirectory)
    window.setCodeDirectoryForTesting("/missing/viem-syntax-directory")
    XCTAssertEqual(preferences.vimSyntaxDirectory, "/missing/viem-syntax-directory")
    XCTAssertTrue(window.codeDiagnosticsForTesting.contains("unavailable"))
    preferences.reportLoadDiagnostics(["Unsupported syntax instruction in sample.vim:12"])
    XCTAssertTrue(window.codeDiagnosticsForTesting.contains("sample.vim:12"))
    window.restoreCodeDirectoryForTesting()
    XCTAssertEqual(window.codeDirectoryForTesting, EVCodePreferences.defaultVimSyntaxDirectory)
    window.close()
  }
}
