import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVThemeSelectionTests: XCTestCase {
  private final class Owner: NSObject, EVApplicationCommandRouting {
    func newDocument(_ sender: Any?) {}
    func openDocument(_ sender: Any?) {}
    func openRecentDocument(_ sender: Any?) {}
    func clearRecentDocuments(_ sender: Any?) {}
    func showSettings(_ sender: Any?) {}
    func showThemeSettings(_ sender: Any?) {}
    func showHelpItem(_ sender: Any?) {}
  }

  private func configuration() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-theme-ui-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory)
  }

  func testStyleThemeMenuUsesCatalogueOrderAndSelectsAndCreatesThemes() throws {
    let configuration = configuration()
    let store = EVThemeStore(configuration: configuration)
    try configuration.createTheme(named: "Zebra")
    try configuration.createTheme(named: "amber")
    let owner = Owner()
    let builder = EVMenuBuilder(owner: owner, themeStore: store)
    let main = builder.buildMainMenu(for: .shared)
    let styles = try XCTUnwrap(main.item(withTitle: "Style")?.submenu)
    XCTAssertEqual(styles.items.first?.title, "Theme")
    let themes = try XCTUnwrap(styles.item(withTitle: "Theme")?.submenu)
    XCTAssertEqual(themes.items.map { $0.isSeparatorItem ? "-" : $0.title }, ["amber", "Midnight", "Paper", "Zebra", "-", "Default", "New Theme…", "Theme Settings…"])
    let fallback = try XCTUnwrap(themes.item(withTitle: "Default"))
    XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(fallback.action), to: fallback.target, from: fallback))
    XCTAssertNil(configuration.currentThemeName)
    builder.menuNeedsUpdate(themes)
    XCTAssertEqual(themes.item(withTitle: "Default")?.state, .on)
    builder.themeActions.requestName = { _ in "Custom" }
    let create = try XCTUnwrap(themes.item(withTitle: "New Theme…"))
    XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(create.action), to: create.target, from: create))
    XCTAssertEqual(configuration.currentThemeName, "Custom")
    builder.menuNeedsUpdate(themes)
    XCTAssertEqual(themes.item(withTitle: "Custom")?.state, .on)
  }

  func testThemeSettingsUsesDropdownAndNewDeleteWithoutPresets() throws {
    let configuration = configuration()
    let store = EVThemeStore(configuration: configuration)
    let controller = EVSettingsWindowController(store: store)
    defer { controller.close() }
    let root = try XCTUnwrap(controller.window?.contentView)
    let popup = try XCTUnwrap(descendants(root).compactMap { $0 as? NSPopUpButton }.first { $0.accessibilityLabel() == "Theme" })
    let buttons = descendants(root).compactMap { $0 as? NSButton }.filter { !($0 is NSPopUpButton) }
    XCTAssertFalse(buttons.contains { ["Paper", "Midnight", "Restore Defaults"].contains($0.title) })
    let create = try XCTUnwrap(buttons.first { $0.title == "New Theme…" })
    let remove = try XCTUnwrap(buttons.first { $0.title == "Delete Theme" })
    controller.themeActions.requestName = { _ in "Custom" }
    create.performClick(nil)
    XCTAssertEqual(configuration.currentThemeName, "Custom")
    XCTAssertEqual(popup.titleOfSelectedItem, "Custom")
    let file = try XCTUnwrap(configuration.selectedThemeURL)
    remove.performClick(nil)
    XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    XCTAssertNil(configuration.currentThemeName)
    XCTAssertEqual(popup.titleOfSelectedItem, "Default")
    XCTAssertFalse(remove.isEnabled)
  }

  func testFileNamedDefaultRemainsDistinctFromBuiltInDefault() throws {
    let configuration = configuration()
    try FileManager.default.copyItem(at: XCTUnwrap(configuration.selectedThemeURL),
      to: configuration.themesDirectory.appendingPathComponent("Default.json"))
    try configuration.selectTheme(named: "Default")
    let controller = EVSettingsWindowController(store: EVThemeStore(configuration: configuration))
    defer { controller.close() }
    let popup = try XCTUnwrap(descendants(XCTUnwrap(controller.window?.contentView)).compactMap { $0 as? NSPopUpButton }.first { $0.accessibilityLabel() == "Theme" })
    let defaults = popup.itemArray.filter { $0.title == "Default" }
    XCTAssertEqual(defaults.count, 2)
    XCTAssertEqual((popup.selectedItem?.representedObject as? EVThemeChoice)?.name, "Default")
    popup.select(try XCTUnwrap(defaults.first { $0.representedObject == nil }))
    XCTAssertTrue(popup.sendAction(popup.action, to: popup.target))
    XCTAssertNil(configuration.currentThemeName)
    popup.select(try XCTUnwrap(popup.itemArray.first { ($0.representedObject as? EVThemeChoice)?.name == "Default" }))
    XCTAssertTrue(popup.sendAction(popup.action, to: popup.target))
    XCTAssertEqual(configuration.currentThemeName, "Default")
  }

  func testIdenticallyDisplayedFilenamesRemainIndividuallySelectable() throws {
    let configuration = configuration()
    let source = try XCTUnwrap(configuration.selectedThemeURL)
    for filename in ["Ocean", "Ocean.json"] {
      try FileManager.default.copyItem(at: source, to: configuration.themesDirectory.appendingPathComponent(filename))
    }
    let owner = Owner()
    let store = EVThemeStore(configuration: configuration)
    let builder = EVMenuBuilder(owner: owner, themeStore: store)
    let menu = try XCTUnwrap(builder.buildMainMenu(for: .shared).item(withTitle: "Style")?.submenu?.item(withTitle: "Theme")?.submenu)
    let controller = EVSettingsWindowController(store: store)
    defer { controller.close() }
    let popup = try XCTUnwrap(descendants(XCTUnwrap(controller.window?.contentView)).compactMap { $0 as? NSPopUpButton }.first { $0.accessibilityLabel() == "Theme" })
    XCTAssertEqual(menu.items.filter { $0.title == "Ocean" }.count, 2)
    XCTAssertEqual(popup.itemArray.filter { $0.title == "Ocean" }.count, 2)
    for filename in ["Ocean", "Ocean.json"] {
      let item = try XCTUnwrap(menu.items.first { ($0.representedObject as? EVThemeChoice)?.fileName == filename })
      XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(item.action), to: item.target, from: item))
      XCTAssertEqual(configuration.selectedThemeURL?.lastPathComponent, filename)
      XCTAssertEqual((popup.selectedItem?.representedObject as? EVThemeChoice)?.fileName, filename)
      XCTAssertEqual(EVConfigurationStore(directory: configuration.directory).selectedThemeURL?.lastPathComponent, filename)
    }
  }

  func testAppearanceStoresTrackSelectionAndLiveEditsForExistingAndFutureConsumers() throws {
    let configuration = configuration()
    let first = EVThemeStore(configuration: configuration)
    let second = EVThemeStore(configuration: configuration)
    XCTAssertEqual(first.theme, .midnight)
    try configuration.selectTheme(named: "Paper")
    XCTAssertEqual(first.theme, .paper)
    XCTAssertEqual(second.theme, .paper)
    var edited = first.theme
    edited.statusFontSize = 17
    first.update(edited)
    XCTAssertEqual(second.theme, edited)
    XCTAssertEqual(EVThemeStore(configuration: configuration).theme, edited)
    try configuration.selectTheme(named: nil)
    let generation = first.generation
    edited = first.theme
    edited.foreground = EVThemeColor(0.2, 0.3, 0.4)
    first.update(edited)
    XCTAssertGreaterThan(first.generation, generation)
    XCTAssertEqual(second.theme, edited)
    XCTAssertEqual(EVThemeStore(configuration: EVConfigurationStore(directory: configuration.directory)).theme, .midnight)
  }

  func testReopeningSettingsRefreshesExternalCatalogueChanges() throws {
    let configuration = configuration()
    let controller = EVSettingsWindowController(store: EVThemeStore(configuration: configuration))
    controller.showWindow(nil)
    controller.close()
    defer { controller.close() }
    let selected = try XCTUnwrap(configuration.selectedThemeURL)
    try FileManager.default.copyItem(at: selected, to: configuration.themesDirectory.appendingPathComponent("Added.json"))
    try FileManager.default.removeItem(at: selected)
    controller.showWindow(nil)
    let popup = try XCTUnwrap(descendants(XCTUnwrap(controller.window?.contentView)).compactMap { $0 as? NSPopUpButton }.first { $0.accessibilityLabel() == "Theme" })
    XCTAssertTrue(popup.itemTitles.contains("Added"))
    XCTAssertFalse(popup.itemTitles.contains("Midnight"))
    XCTAssertEqual(popup.titleOfSelectedItem, "Default")
    XCTAssertNil(configuration.currentThemeName)
  }

  func testOpeningThemeMenuAfterExternalRemovalFallsBackToDefault() throws {
    let configuration = configuration()
    let owner = Owner()
    let builder = EVMenuBuilder(owner: owner, themeStore: EVThemeStore(configuration: configuration))
    let main = builder.buildMainMenu(for: .shared)
    let menu = try XCTUnwrap(main.item(withTitle: "Style")?.submenu?.item(withTitle: "Theme")?.submenu)
    try FileManager.default.removeItem(at: XCTUnwrap(configuration.selectedThemeURL))
    builder.menuNeedsUpdate(menu)
    XCTAssertNil(configuration.currentThemeName)
    XCTAssertEqual(menu.item(withTitle: "Default")?.state, .on)
  }

  private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
