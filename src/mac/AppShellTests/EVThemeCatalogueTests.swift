import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVThemeCatalogueTests: XCTestCase {
  private func directory() -> URL {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-themes-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return directory
  }

  private func object(_ file: URL) throws -> [String: Any] {
    try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
  }

  func testNewProfileSeedsFallbackThemesWithoutResourcesAndSelectsMidnight() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory, bundleResourceURL: nil)
    XCTAssertNil(store.lastError)
    XCTAssertEqual(store.availableThemeNames, ["Midnight", "Paper"])
    XCTAssertEqual(store.currentThemeName, "Midnight")
    XCTAssertEqual(store.theme, .midnight)
    XCTAssertEqual(try object(directory.appendingPathComponent("config.json"))["selectedTheme"] as? String, "Midnight")
    for name in ["Paper", "Midnight"] {
      let file = try object(store.themesDirectory.appendingPathComponent(name + ".json"))
      let styles = try XCTUnwrap(file["styles"] as? [String: Any])
      XCTAssertTrue(Set(["text", "markdown", "code"]).isSubset(of: Set(styles.keys)))
    }
  }

  func testBundledThemesRestoreOnlyMissingFilesAndRevertFromCurrentPackage() throws {
    let resources = directory()
    let themes = resources.appendingPathComponent("themes")
    try FileManager.default.createDirectory(at: themes, withIntermediateDirectories: true)
    var repo = URL(fileURLWithPath: #filePath)
    for _ in 0..<4 { repo.deleteLastPathComponent() }
    let names = ["Midnight", "Midnight Mono", "Midnight Proportional", "Paper", "Typewriter"]
    var bundled: [String: Data] = [:]
    for name in names {
      let data = try Data(contentsOf: repo.appendingPathComponent("assets/themes/\(name).json"))
      _ = try EVThemeFile.decode(data)
      bundled[name] = data
      try data.write(to: themes.appendingPathComponent(name + ".json"))
    }
    let profile = directory()
    let store = EVConfigurationStore(directory: profile, bundleResourceURL: resources)
    XCTAssertNil(store.lastError)
    XCTAssertEqual(store.availableThemeNames, names)
    XCTAssertEqual(store.currentThemeName, "Midnight")
    for name in names {
      let installed = store.themesDirectory.appendingPathComponent(name + ".json")
      XCTAssertEqual(try Data(contentsOf: installed), bundled[name])
      try store.selectTheme(named: name)
      XCTAssertEqual(store.currentThemeName, name)
      XCTAssertEqual(try Data(contentsOf: installed), bundled[name])
    }
    try store.selectTheme(named: "Midnight")
    let midnight = store.themesDirectory.appendingPathComponent("Midnight.json")
    let customized = try XCTUnwrap(bundled["Midnight"]) + Data("\n\n".utf8)
    try customized.write(to: midnight)
    for name in names where name != "Midnight" {
      try FileManager.default.removeItem(at: store.themesDirectory.appendingPathComponent(name + ".json"))
    }
    let reopened = EVConfigurationStore(directory: profile, bundleResourceURL: resources)
    XCTAssertNil(reopened.lastError)
    XCTAssertEqual(reopened.availableThemeNames, ["Midnight"])
    XCTAssertEqual(try Data(contentsOf: midnight), customized)
    let config = try Data(contentsOf: profile.appendingPathComponent("config.json"))
    try reopened.restoreMissingBundledThemes()
    XCTAssertEqual(reopened.availableThemeNames, names)
    XCTAssertEqual(try Data(contentsOf: midnight), customized)
    XCTAssertEqual(try Data(contentsOf: profile.appendingPathComponent("config.json")), config)
    for name in names where name != "Midnight" {
      XCTAssertEqual(try Data(contentsOf: reopened.themesDirectory.appendingPathComponent(name + ".json")), bundled[name])
    }
    XCTAssertTrue(reopened.currentThemeIsBundled)
    XCTAssertThrowsError(try reopened.deleteCurrentTheme())
    try reopened.setTheme(.paper)
    try reopened.saveStyleDefaults(Data(#"{"version":1,"block_styles":[],"character_styles":[],"future":42}"#.utf8), named: "markdown")
    try reopened.revertCurrentTheme()
    XCTAssertEqual(try Data(contentsOf: midnight), bundled["Midnight"])
    XCTAssertEqual(reopened.currentThemeFileName, "Midnight.json")
    XCTAssertEqual(reopened.theme, .midnight)
    XCTAssertEqual(try Data(contentsOf: profile.appendingPathComponent("config.json")), config)
    let current = reopened.theme
    try Data("invalid bundled theme".utf8).write(to: themes.appendingPathComponent("Midnight.json"))
    XCTAssertThrowsError(try reopened.revertCurrentTheme())
    XCTAssertEqual(reopened.theme, current)
    XCTAssertEqual(try Data(contentsOf: midnight), bundled["Midnight"])
  }

  func testDefaultChangesStayInMemoryAndNewThemeCopiesThem() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory)
    try store.selectTheme(named: nil)
    let config = try Data(contentsOf: directory.appendingPathComponent("config.json"))
    var appearance = store.theme
    appearance.foreground = EVThemeColor(0.12, 0.34, 0.56)
    appearance.statusFontSize = 17
    try store.setTheme(appearance)
    var text = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(store.styleDefaults(named: "text"))) as? [String: Any])
    text["futureSetting"] = 42
    try store.saveStyleDefaults(JSONSerialization.data(withJSONObject: text), named: "text")
    XCTAssertEqual(store.theme, appearance)
    XCTAssertNil(store.selectedThemeURL)
    XCTAssertEqual(try Data(contentsOf: directory.appendingPathComponent("config.json")), config)
    XCTAssertEqual(store.availableThemeNames, ["Midnight", "Paper"])
    XCTAssertEqual(EVConfigurationStore(directory: directory).theme, .midnight)

    try store.createTheme(named: "My Theme")
    XCTAssertEqual(store.currentThemeName, "My Theme")
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertEqual(reopened.currentThemeName, "My Theme")
    XCTAssertEqual(reopened.theme, appearance)
    let restored = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(reopened.styleDefaults(named: "text"))) as? [String: Any])
    XCTAssertEqual(restored["futureSetting"] as? Int, 42)
    XCTAssertFalse(FileManager.default.fileExists(atPath: directory.appendingPathComponent("text_style.json").path))
  }

  func testCatalogueReadsFilesystemAndSortsCaseInsensitively() throws {
    let store = EVConfigurationStore(directory: directory())
    for name in ["zebra", "Alpha", "beta"] { try store.createTheme(named: name) }
    let external = store.themesDirectory.appendingPathComponent("aardvark.theme")
    try EVThemeFile.encode(EVThemeFile.builtin(paper: true)).write(to: external)
    try FileManager.default.createDirectory(at: store.themesDirectory.appendingPathComponent("Not a file.json"), withIntermediateDirectories: false)
    XCTAssertEqual(store.availableThemeNames, ["aardvark.theme", "Alpha", "beta", "Midnight", "Paper", "zebra"])
    try store.selectTheme(named: "aardvark.theme")
    XCTAssertEqual(store.selectedThemeURL?.resolvingSymlinksInPath(), external.resolvingSymlinksInPath())
    XCTAssertEqual(store.theme, .paper)
  }

  func testInvalidAndDuplicateNamesCannotOverwriteThemes() throws {
    let store = EVConfigurationStore(directory: directory())
    let original = try Data(contentsOf: XCTUnwrap(store.selectedThemeURL))
    for name in ["", "Default", "default", "../outside", "a/b", "a\\b", "a:b", "a?b", "CON", "LPT1", ".", "..", "Name.", "Name ", "a\n", String(repeating: "a", count: 33), "midnight"] {
      XCTAssertThrowsError(try store.createTheme(named: name), name)
    }
    XCTAssertEqual(store.currentThemeName, "Midnight")
    XCTAssertEqual(try Data(contentsOf: XCTUnwrap(store.selectedThemeURL)), original)
    try store.createTheme(named: String(repeating: "a", count: 32))
    XCTAssertEqual(store.currentThemeName?.count, 32)
  }

  func testMissingAndMalformedSelectedThemesUseDefaultWithoutChangingFiles() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory)
    let selected = try XCTUnwrap(store.selectedThemeURL)
    let malformed = Data("{invalid".utf8)
    try malformed.write(to: selected, options: .atomic)
    let invalid = EVConfigurationStore(directory: directory)
    XCTAssertNil(invalid.currentThemeName)
    XCTAssertEqual(invalid.theme, .midnight)
    XCTAssertNotNil(invalid.lastError)
    XCTAssertEqual(try Data(contentsOf: selected), malformed)
    try invalid.setSmartQuotes(true)
    try FileManager.default.removeItem(at: selected)
    let missing = EVConfigurationStore(directory: directory)
    XCTAssertNil(missing.currentThemeName)
    XCTAssertEqual(missing.theme, .midnight)
    XCTAssertTrue(missing.smartQuotes)
    try store.reloadCurrentTheme()
    XCTAssertNil(store.currentThemeName)
    XCTAssertEqual(store.theme, .midnight)
  }

  func testDeletingSelectedThemeFallsBackAndDoesNotReseedIt() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory)
    try store.createTheme(named: "Custom")
    XCTAssertFalse(store.currentThemeIsBundled)
    try store.deleteCurrentTheme()
    XCTAssertNil(store.currentThemeName)
    XCTAssertEqual(store.theme, .midnight)
    XCTAssertEqual(store.availableThemeNames, ["Midnight", "Paper"])
    XCTAssertEqual(EVConfigurationStore(directory: directory).availableThemeNames, ["Midnight", "Paper"])
  }

  func testBundledIdentityUsesTheFilenameNotItsDisplayName() throws {
    let store = EVConfigurationStore(directory: directory())
    let file = store.themesDirectory.appendingPathComponent("Paper")
    try EVThemeFile.encode(EVThemeFile.builtin()).write(to: file)
    try store.selectTheme(named: "Paper", fileName: "Paper")
    XCTAssertFalse(store.currentThemeIsBundled)
    XCTAssertThrowsError(try store.revertCurrentTheme())
    try store.deleteCurrentTheme()
    XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    XCTAssertTrue(FileManager.default.fileExists(atPath: store.themesDirectory.appendingPathComponent("Paper.json").path))
  }

  func testLegacyAppearanceAndStyleFilesAreIgnoredWithoutCreatingATheme() throws {
    let directory = directory()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let appearance = try JSONSerialization.jsonObject(with: JSONEncoder().encode(EVTheme.paper))
    let config = try JSONSerialization.data(withJSONObject: ["version": 1, "theme": appearance, "editing": ["smartQuotes": true]])
    let configURL = directory.appendingPathComponent("config.json")
    try config.write(to: configURL)
    var legacyFiles: [URL: Data] = [:]
    for name in ["text", "markdown", "code"] {
      let style = try JSONSerialization.data(withJSONObject: ["version": name == "code" ? 2 : 1,
        "block_styles": [], "character_styles": [], "future": 42])
      let file = directory.appendingPathComponent("\(name)_style.json")
      try style.write(to: file)
      legacyFiles[file] = style
    }

    let store = EVConfigurationStore(directory: directory)
    XCTAssertNil(store.lastError)
    XCTAssertNil(store.currentThemeName)
    XCTAssertEqual(store.theme, .midnight)
    XCTAssertTrue(store.smartQuotes)
    XCTAssertTrue(store.availableThemeNames.isEmpty)
    let builtinStyles = try XCTUnwrap(EVThemeFile.builtin()["styles"] as? [String: Any])
    for name in ["text", "markdown", "code"] {
      let expected = try JSONSerialization.data(withJSONObject: XCTUnwrap(builtinStyles[name]), options: [.sortedKeys])
      XCTAssertEqual(try store.styleDefaults(named: name), expected)
    }
    XCTAssertEqual(try Data(contentsOf: configURL), config)
    for (file, data) in legacyFiles { XCTAssertEqual(try Data(contentsOf: file), data) }
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertNil(reopened.currentThemeName)
    XCTAssertTrue(reopened.availableThemeNames.isEmpty)
  }

  func testMalformedLegacySettingsDoNotBlockCurrentThemeOrPreferenceUpdates() throws {
    let directory = directory()
    let themes = directory.appendingPathComponent("themes")
    try FileManager.default.createDirectory(at: themes, withIntermediateDirectories: true)
    let themeURL = themes.appendingPathComponent("Current.json")
    let theme = try EVThemeFile.encode(EVThemeFile.builtin(paper: true))
    try theme.write(to: themeURL)
    let configURL = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"theme":"obsolete","selectedTheme":"Current","selectedThemeFile":"Current.json"}"#.utf8)
      .write(to: configURL)
    let invalid = Data("{invalid".utf8)
    for name in ["text", "markdown", "code"] {
      try invalid.write(to: directory.appendingPathComponent("\(name)_style.json"))
    }

    let store = EVConfigurationStore(directory: directory)
    XCTAssertNil(store.lastError)
    XCTAssertEqual(store.currentThemeName, "Current")
    XCTAssertEqual(store.theme, .paper)
    XCTAssertEqual(store.availableThemeNames, ["Current"])
    try store.setSmartQuotes(true)
    try store.selectTheme(named: "Current")
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertNil(reopened.lastError)
    XCTAssertTrue(reopened.smartQuotes)
    XCTAssertEqual(reopened.theme, .paper)
    XCTAssertEqual(try object(configURL)["theme"] as? String, "obsolete")
    XCTAssertEqual(try Data(contentsOf: themeURL), theme)
    for name in ["text", "markdown", "code"] {
      XCTAssertEqual(try Data(contentsOf: directory.appendingPathComponent("\(name)_style.json")), invalid)
    }
  }

  func testIndependentThemeWritesPreserveOtherSectionsAndFailedWriteDoesNotPublish() throws {
    let directory = directory()
    let first = EVConfigurationStore(directory: directory)
    let second = EVConfigurationStore(directory: directory)
    try first.setTheme(.paper)
    let json = Data(#"{"version":1,"block_styles":[],"character_styles":[],"future":91}"#.utf8)
    try second.saveStyleDefaults(json, named: "markdown")
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertEqual(reopened.theme, .paper)
    XCTAssertEqual(try object(XCTUnwrap(reopened.selectedThemeURL))["version"] as? Int, 1)
    let file = try XCTUnwrap(second.selectedThemeURL)
    let invalid = Data("broken".utf8)
    try invalid.write(to: file, options: .atomic)
    let previous = second.theme
    XCTAssertThrowsError(try second.setTheme(.midnight))
    XCTAssertEqual(second.theme, previous)
    XCTAssertEqual(try Data(contentsOf: file), invalid)
  }

  func testStylePropertyMapsReplaceWholeValuesWhileUnknownFieldsSurvive() throws {
    let store = EVConfigurationStore(directory: directory())
    var sheet = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(store.styleDefaults(named: "text"))) as? [String: Any])
    var blocks = try XCTUnwrap(sheet["block_styles"] as? [[String: Any]])
    let index = try XCTUnwrap(blocks.firstIndex { $0["id"] as? String == "Paragraph" })
    var character = try XCTUnwrap(blocks[index]["character"] as? [String: Any])
    var paragraph = try XCTUnwrap(blocks[index]["block"] as? [String: Any])
    character["open_type_features"] = ["liga": 0, "ss01": 1]
    character["futureTypography"] = "preserved"
    paragraph["line_spacing"] = ["Multiplier": 1.5]
    blocks[index]["character"] = character
    blocks[index]["block"] = paragraph
    sheet["block_styles"] = blocks
    try store.saveStyleDefaults(JSONSerialization.data(withJSONObject: sheet), named: "text")
    character["open_type_features"] = ["kern": 0]
    character.removeValue(forKey: "futureTypography")
    paragraph["line_spacing"] = ["Exact": 20]
    blocks[index]["character"] = character
    blocks[index]["block"] = paragraph
    sheet["block_styles"] = blocks
    try store.saveStyleDefaults(JSONSerialization.data(withJSONObject: sheet), named: "text")
    let saved = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(store.styleDefaults(named: "text"))) as? [String: Any])
    let savedBlocks = try XCTUnwrap(saved["block_styles"] as? [[String: Any]])
    let savedCharacter = try XCTUnwrap(savedBlocks[index]["character"] as? [String: Any])
    XCTAssertEqual(savedCharacter["open_type_features"] as? [String: Int], ["kern": 0])
    XCTAssertEqual(savedCharacter["futureTypography"] as? String, "preserved")
    XCTAssertEqual((savedBlocks[index]["block"] as? [String: Any])?["line_spacing"] as? [String: Int], ["Exact": 20])
  }

  func testOmittedStyleFamiliesUseBuiltinsAndDotPrefixedThemesRemainSelectable() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory)
    let expected = try store.styleDefaults(named: "text")
    var sparse = try EVThemeFile.builtin()
    sparse.removeValue(forKey: "styles")
    try EVThemeFile.encode(sparse).write(to: store.themesDirectory.appendingPathComponent("Sparse.json"))
    try store.selectTheme(named: "Sparse")
    XCTAssertEqual(try store.styleDefaults(named: "text"), expected)
    XCTAssertNotNil(try store.codeStyleSheet())
    try store.createTheme(named: ".night")
    XCTAssertTrue(store.availableThemeNames.contains(".night"))
    XCTAssertEqual(EVConfigurationStore(directory: directory).currentThemeName, ".night")
  }

  func testSameDisplayNamesRetainTheSelectedFilenameAcrossLaunches() throws {
    let directory = directory()
    let store = EVConfigurationStore(directory: directory)
    try EVThemeFile.encode(EVThemeFile.builtin()).write(to: store.themesDirectory.appendingPathComponent("Ocean"))
    let selected = store.themesDirectory.appendingPathComponent("Ocean.json")
    try EVThemeFile.encode(EVThemeFile.builtin(paper: true)).write(to: selected)
    XCTAssertEqual(store.availableThemes.filter { $0.name == "Ocean" }.count, 2)
    try store.selectTheme(named: "Ocean", fileName: "Ocean.json")
    XCTAssertEqual(store.theme, .paper)
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertEqual(reopened.currentThemeFileName, "Ocean.json")
    XCTAssertEqual(reopened.theme, .paper)
    try FileManager.default.removeItem(at: selected)
    try reopened.reloadFromDisk()
    XCTAssertNil(reopened.currentThemeName)
    XCTAssertEqual(reopened.theme, .midnight)
    XCTAssertNil(EVConfigurationStore(directory: directory).currentThemeName)
  }

  func testPreferenceNotificationsDoNotRepublishThemesOrDiscardDefaultEdits() throws {
    let store = EVConfigurationStore(directory: directory())
    let preferences = EVEditingPreferences(configuration: store)
    var publications = 0
    let observer = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: nil, queue: .main) { notification in
      MainActor.assumeIsolated {
        if notification.object as? EVConfigurationStore === store { publications += 1 }
      }
    }
    defer { NotificationCenter.default.removeObserver(observer) }
    try store.setTheme(.paper)
    XCTAssertEqual(publications, 1)
    preferences.setSmartQuotes(true)
    XCTAssertTrue(preferences.smartQuotes)
    XCTAssertEqual(publications, 1)
    try store.selectTheme(named: nil)
    try store.setTheme(.paper)
    XCTAssertEqual(publications, 3)
    preferences.setSmartQuotes(false)
    try store.reloadFromDisk()
    XCTAssertEqual(publications, 3)
    XCTAssertEqual(store.theme, .paper)
    XCTAssertNil(store.currentThemeName)
  }

  func testMalformedThemeDoesNotBlockUnrelatedEditingPreferences() throws {
    let store = EVConfigurationStore(directory: directory())
    let preferences = EVEditingPreferences(configuration: store)
    let selected = try XCTUnwrap(store.selectedThemeURL)
    let malformed = Data("invalid theme".utf8)
    try malformed.write(to: selected)
    preferences.setSmartQuotes(true)
    XCTAssertTrue(preferences.smartQuotes)
    XCTAssertTrue(store.smartQuotes)
    XCTAssertEqual(store.theme, .midnight)
    XCTAssertNotNil(store.lastError)
    XCTAssertEqual(try Data(contentsOf: selected), malformed)
    XCTAssertThrowsError(try store.reloadCurrentTheme())
  }
}
