import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVCodePreferencesTests: XCTestCase {
  private func fixture() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory,
      bundleResourceURL: directory.appendingPathComponent("Moved Viem – Résumé.app/Contents/Resources"))
  }

  func testBundledResourcesFollowRelocationWithoutPersistingPaths() throws {
    let configuration = fixture()
    XCTAssertTrue(configuration.bundledVimSyntaxDirectory.hasSuffix("Moved Viem – Résumé.app/Contents/Resources/vim/runtime/syntax"))
    XCTAssertFalse(FileManager.default.fileExists(atPath: configuration.directory.appendingPathComponent("config.json").path))
    try configuration.setSmartQuotes(true)
    let file = configuration.directory.appendingPathComponent("config.json")
    let stored = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    XCTAssertNil((stored["code"] as? [String: Any])?["vimSyntaxDirectory"])
    let movedResources = configuration.directory.appendingPathComponent("Other Folder/Éditeur.app/Contents/Resources")
    let reopened = EVConfigurationStore(directory: configuration.directory, bundleResourceURL: movedResources)
    let expected = movedResources.appendingPathComponent("vim/runtime/syntax").path
    XCTAssertEqual(reopened.bundledVimSyntaxDirectory, expected)
    let previousDirectory = FileManager.default.currentDirectoryPath
    defer { _ = FileManager.default.changeCurrentDirectoryPath(previousDirectory) }
    XCTAssertTrue(FileManager.default.changeCurrentDirectoryPath(configuration.directory.path))
    XCTAssertEqual(reopened.bundledVimSyntaxDirectory, expected)
    XCTAssertNotNil(EVCodePreferences(configuration: reopened).bundledSyntaxDiagnostic)
    try FileManager.default.createDirectory(atPath: expected, withIntermediateDirectories: true)
    XCTAssertNil(EVCodePreferences(configuration: reopened).bundledSyntaxDiagnostic)
  }

  func testRetiredSyntaxDirectoryValuesAreIgnoredAndRemovedOnSettingsWrite() throws {
    let configuration = fixture()
    try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
    let file = configuration.directory.appendingPathComponent("config.json")
    let resources = configuration.directory.appendingPathComponent("Resources")
    let oldValues: [Any] = ["/custom/syntax", "", NSNull(), false]
    for oldValue in oldValues {
      let original = try JSONSerialization.data(withJSONObject: ["version": 1,
        "code": ["vimSyntaxDirectory": oldValue, "future": 42]])
      try original.write(to: file)
      let reopened = EVConfigurationStore(directory: configuration.directory, bundleResourceURL: resources)
      XCTAssertNil(reopened.lastError)
      XCTAssertEqual(reopened.bundledVimSyntaxDirectory, resources.appendingPathComponent("vim/runtime/syntax").path)
      XCTAssertEqual(try Data(contentsOf: file), original, "Loading a profile must not rewrite it")
      try reopened.setSmartQuotes(true)
      let stored = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
      XCTAssertNil((stored["code"] as? [String: Any])?["vimSyntaxDirectory"])
      XCTAssertEqual((stored["code"] as? [String: Any])?["future"] as? Int, 42)
    }
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
    let old = Data(#"{"version":2,"block_styles":[{"id":"Paragraph","character":{"size":19}}],"character_styles":[{"id":"keyword"}]}"#.utf8)
    let cleared = Data(#"{"version":2,"block_styles":[{"id":"Paragraph","character":{}}],"character_styles":[],"suppressed":["keyword"]}"#.utf8)
    try configuration.saveCodeStyleSheet(old)
    try configuration.saveCodeStyleSheet(cleared)
    let stored = try XCTUnwrap(configuration.codeStyleSheet())
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: stored) as? [String: Any])
    let blocks = try XCTUnwrap(object["block_styles"] as? [[String: Any]])
    XCTAssertNil((blocks[0]["character"] as? [String: Any])?["size"])
    XCTAssertEqual((object["character_styles"] as? [[String: Any]])?.count, 0)
    XCTAssertNil(try configuration.styleDefaults(named: "text"))
  }

  func testCodeStyleVersionTwoLoadsAndSavesAcrossSettingsUndo() throws {
    let configuration = fixture()
    let customized = Data(#"{"version":2,"character_styles":[{"id":"syntax:@comment","properties":{"foreground":{"red":0.4}}}]}"#.utf8)
    let linked = Data(#"{"version":2,"character_styles":[],"suppressed_character_ids":["syntax:Todo"]}"#.utf8)
    for (data, expectedCount) in [(customized, 1), (linked, 0), (customized, 1), (linked, 0)] {
      try configuration.saveCodeStyleSheet(data)
      let reopened = EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil)
      let saved = try XCTUnwrap(reopened.codeStyleSheet())
      XCTAssertEqual(try reopened.styleDefaults(named: "code"), saved)
      let object = try XCTUnwrap(JSONSerialization.jsonObject(with: saved) as? [String: Any])
      XCTAssertEqual(object["version"] as? Int, 2)
      XCTAssertEqual((object["character_styles"] as? [[String: Any]])?.count, expectedCount)
    }
    // The generic format entry point must retain Code's complete replacement
    // semantics, including declarations removed during settings undo.
    try configuration.saveStyleDefaults(customized, named: "code")
    try configuration.saveStyleDefaults(linked, named: "code")
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(configuration.codeStyleSheet())) as? [String: Any])
    XCTAssertEqual((object["character_styles"] as? [[String: Any]])?.count, 0)
    XCTAssertEqual(object["suppressed_character_ids"] as? [String], ["syntax:Todo"])
    XCTAssertThrowsError(try configuration.saveStyleDefaults(linked, named: "html"))
  }

  func testUnsupportedCodeStyleVersionsStayOnDiskUntilExplicitRestore() throws {
    let configuration = fixture()
    let valid = Data(#"{"version":2,"character_styles":[]}"#.utf8)
    try configuration.saveCodeStyleSheet(valid)
    let file = configuration.directory.appendingPathComponent("code_style.json")
    for version in ["0", "1", "3", "2.5", "true", "\"2\""] {
      let original = try Data(contentsOf: file)
      let unsupported = Data("{\"version\":\(version)}".utf8)
      XCTAssertThrowsError(try configuration.saveCodeStyleSheet(unsupported))
      XCTAssertEqual(try Data(contentsOf: file), original)
      try unsupported.write(to: file)
      XCTAssertThrowsError(try configuration.codeStyleSheet())
      XCTAssertThrowsError(try configuration.saveCodeStyleSheet(valid))
      XCTAssertEqual(try Data(contentsOf: file), unsupported)
      try configuration.saveCodeStyleSheet(valid, replacingInvalidFile: true)
      XCTAssertNotNil(try configuration.codeStyleSheet())
    }
  }

  func testCodeSettingsPageReportsBundledResourceAndProviderDiagnostics() throws {
    let configuration = fixture()
    let preferences = EVCodePreferences(configuration: configuration)
    let window = EVSettingsWindowController(store: EVThemeStore(configuration: configuration),
      editingPreferences: EVEditingPreferences(configuration: configuration), codePreferences: preferences)
    window.showCodeCategoryForTesting()
    XCTAssertTrue(window.codeDiagnosticsForTesting.contains("unavailable"))
    preferences.reportLoadDiagnostics(["Unsupported syntax instruction in sample.vim:12"])
    XCTAssertTrue(window.codeDiagnosticsForTesting.contains("sample.vim:12"))
    window.close()
  }
}
