import AppKit
import XCTest

@testable import ViemAppShell

@MainActor
final class EVThemeTests: XCTestCase {
  func testThemePersistsSeparatelyAndRejectsInvalidSettings() throws {
    let name = "com.viem.tests.theme.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: name) }
    let center = NotificationCenter()
    let store = EVThemeStore(configuration: configuration, center: center)
    var notifications = 0
    let observer = center.addObserver(forName: .viemThemeDidChange, object: nil, queue: .main) {
      _ in notifications += 1
    }
    defer { center.removeObserver(observer) }
    var theme = EVTheme.midnight
    theme.caret = EVThemeColor(0.2, 0.3, 0.4)
    store.update(theme)
    XCTAssertEqual(notifications, 1)
    XCTAssertEqual(EVThemeStore(configuration: EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)).theme, theme)
    let generation = store.generation
    store.update(theme)
    var invalid = theme
    invalid.foreground.red = .nan
    store.update(invalid)
    invalid = theme
    invalid.background.alpha = -1
    store.update(invalid)
    XCTAssertEqual(store.generation, generation)
    XCTAssertEqual(store.theme, theme)
    XCTAssertEqual(notifications, 1)
  }

  func testMalformedSavedThemeFallsBackAndSettingsWindowFits() throws {
    let name = "com.viem.tests.theme.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: name) }
    try Data("invalid json".utf8).write(to: XCTUnwrap(configuration.selectedThemeURL))
    let reopened = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    XCTAssertNotNil(reopened.lastError)
    let store = EVThemeStore(configuration: reopened)
    XCTAssertEqual(store.theme, .midnight)
    let settings = EVSettingsWindowController(store: store)
    defer { settings.close() }
    settings.showWindow(nil)
    let window = try XCTUnwrap(settings.window)
    window.contentView?.layoutSubtreeIfNeeded()
    XCTAssertEqual(window.title, "Viem Settings")
    XCTAssertGreaterThanOrEqual(window.contentLayoutRect.width, 760)
    XCTAssertGreaterThanOrEqual(window.contentLayoutRect.height, 640)
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    XCTAssertGreaterThanOrEqual(window.contentLayoutRect.height, 640)
    let fields = descendants(window.contentView).compactMap { $0 as? NSTextField }
    XCTAssertNil(fields.first { $0.accessibilityLabel() == "Status font size" })
    XCTAssertFalse(fields.contains { $0.accessibilityLabel()?.contains("margin") == true })
    let categories = try XCTUnwrap(
      descendants(window.contentView).compactMap { $0 as? NSTableView }.first)
    XCTAssertGreaterThan(
      categories.convert(categories.rect(ofRow: 0), to: nil).midY,
      window.contentLayoutRect.height / 2)
  }

  func testStatusChromeResolvesSparseInheritanceAndIgnoresRemovedAppearanceKeys() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-theme-chrome-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let configuration = EVConfigurationStore(directory: directory)
    let store = EVThemeStore(configuration: configuration)
    try editStatusStyle(configuration, character: ["font_families": ["Georgia"], "size": 18],
      block: ["padding_left": 23, "padding_right": 9, "border_top_width": 3])
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 18)
    XCTAssertEqual(store.chromeStyles.statusBar.padding.left, 23)
    XCTAssertEqual(store.chromeStyles.statusBar.topBorder.width, 3)
    XCTAssertEqual(store.chromeStyles.statusCommand.font.pointSize, 18)
    XCTAssertEqual(store.chromeStyles.statusCommand.topBorder.width, 0)
    XCTAssertEqual(EVThemeStore(configuration: EVConfigurationStore(directory: directory)).chromeStyles.statusBar.font.pointSize, 18)
    let file = try XCTUnwrap(configuration.selectedThemeURL)
    var root = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    var appearance = try XCTUnwrap(root["theme"] as? [String: Any])
    appearance["statusFontSize"] = 999
    appearance["statusFontFamily"] = "missing font"
    root["theme"] = appearance
    try JSONSerialization.data(withJSONObject: root).write(to: file)
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertNil(reopened.lastError)
    XCTAssertEqual(EVThemeStore(configuration: reopened).chromeStyles.statusBar.font.pointSize, 18)
  }

  func testStatusStyleEditsFromMarkdownPublishAllFamiliesAtomically() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-shared-chrome-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let configuration = EVConfigurationStore(directory: directory)
    let store = EVThemeStore(configuration: configuration)
    func blocks(_ family: String) throws -> [[String: Any]] {
      let data = try XCTUnwrap(configuration.styleDefaults(named: family))
      return try XCTUnwrap((JSONSerialization.jsonObject(with: data) as? [String: Any])?["block_styles"] as? [[String: Any]])
    }
    let textBefore = try blocks("text")
    let codeBefore = try blocks("code")
    let markdownBefore = try XCTUnwrap(configuration.styleDefaults(named: "markdown"))
    try editStatusStyle(configuration, family: "markdown", character: ["size": 22], block: ["padding_left": 21])
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 22)
    XCTAssertEqual(store.chromeStyles.statusCommand.font.pointSize, 22)
    for (family, before) in [("text", textBefore), ("code", codeBefore)] {
      let after = try blocks(family)
      let bar = try XCTUnwrap(after.first { $0["id"] as? String == "Status bar" })
      XCTAssertEqual((bar["character"] as? [String: Any])?["size"] as? Int, 22)
      for old in before where !["Status bar", "Status bar command"].contains(old["id"] as? String ?? "") {
        let retained = try XCTUnwrap(after.first { $0["id"] as? String == old["id"] as? String })
        XCTAssertTrue(NSDictionary(dictionary: old).isEqual(to: retained))
      }
    }
    // The existing theme-style undo path restores the originating specimen.
    try configuration.saveStyleDefaults(markdownBefore, named: "markdown")
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 11)
    let file = try XCTUnwrap(configuration.selectedThemeURL)
    let corrupt = Data("broken theme".utf8)
    try corrupt.write(to: file)
    let generation = store.generation
    XCTAssertThrowsError(try editStatusStyle(configuration, family: "markdown", character: ["size": 24]))
    XCTAssertEqual(store.generation, generation)
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 11)
    XCTAssertEqual(try Data(contentsOf: file), corrupt)
  }

  func testCodeDeltaClearingRestoresSharedStatusStyleDefaults() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-chrome-code-delta-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let configuration = EVConfigurationStore(directory: directory)
    let store = EVThemeStore(configuration: configuration)
    try editStatusStyle(configuration, family: "code", character: ["size": 23])
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 23)
    try configuration.saveCodeStyleSheet(Data("{\"version\":3,\"block_styles\":[],\"character_styles\":[],\"suppressed_character_ids\":[]}".utf8))
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 11)
    XCTAssertEqual(store.chromeStyles.statusCommand.font.pointSize, 11)
  }

  func testUnrelatedCodeDeltaEditPreservesSharedStatusStylesWhenPriorDefinitionsAreOmitted() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-chrome-code-omitted-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let configuration = EVConfigurationStore(directory: directory)
    let store = EVThemeStore(configuration: configuration)
    try editStatusStyle(configuration, character: ["size": 23])
    let file = try XCTUnwrap(configuration.selectedThemeURL)
    var root = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    var sheets = try XCTUnwrap(root["styles"] as? [String: Any])
    var code = try XCTUnwrap(sheets["code"] as? [String: Any])
    var blocks = try XCTUnwrap(code["block_styles"] as? [[String: Any]])
    blocks.removeAll { ["Status bar", "Status bar command"].contains($0["id"] as? String ?? "") }
    code["block_styles"] = blocks
    sheets["code"] = code
    root["styles"] = sheets
    try JSONSerialization.data(withJSONObject: root).write(to: file)
    try configuration.reloadCurrentTheme()
    let textBefore = try configuration.styleDefaults(named: "text")
    let markdownBefore = try configuration.styleDefaults(named: "markdown")
    let base = try XCTUnwrap(blocks.firstIndex { $0["id"] as? String == "Paragraph" })
    var character = blocks[base]["character"] as? [String: Any] ?? [:]
    character["size"] = 17
    blocks[base]["character"] = character
    code["block_styles"] = blocks
    try configuration.saveCodeStyleSheet(JSONSerialization.data(withJSONObject: code))
    XCTAssertEqual(store.chromeStyles.statusBar.font.pointSize, 23)
    XCTAssertEqual(store.chromeStyles.statusCommand.font.pointSize, 23)
    XCTAssertEqual(try configuration.styleDefaults(named: "text"), textBefore)
    XCTAssertEqual(try configuration.styleDefaults(named: "markdown"), markdownBefore)
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
