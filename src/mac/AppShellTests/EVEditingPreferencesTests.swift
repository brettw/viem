import AppKit
import XCTest

@testable import ViemAppShell

@MainActor
final class EVEditingPreferencesTests: XCTestCase {
  func testSmartQuotesDefaultsOffPersistsAndNotifiesOnlyOnChanges() throws {
    let suite = "viem-editing-test-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: suite) }
    let center = NotificationCenter()
    let preferences = EVEditingPreferences(configuration: configuration, center: center)
    var changes = 0
    let observer = center.addObserver(
      forName: .viemEditingPreferencesDidChange, object: nil, queue: .main
    ) { _ in changes += 1 }
    defer { center.removeObserver(observer) }
    XCTAssertFalse(preferences.smartQuotes)
    preferences.setSmartQuotes(true)
    XCTAssertTrue(EVEditingPreferences(configuration: EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)).smartQuotes)
    XCTAssertEqual(changes, 1)
    preferences.setSmartQuotes(true)
    XCTAssertEqual(changes, 1)
    preferences.setSmartQuotes(false)
    XCTAssertFalse(EVEditingPreferences(configuration: EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)).smartQuotes)
    XCTAssertEqual(changes, 2)
  }

  func testEditingCategoryCheckboxUpdatesPreferenceWithoutChangingTheme() throws {
    let suite = "viem-editing-settings-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: suite) }
    let preferences = EVEditingPreferences(configuration: configuration)
    let theme = EVThemeStore(configuration: configuration)
    let settings = EVSettingsWindowController(store: theme, editingPreferences: preferences)
    defer { settings.close() }
    settings.showWindow(nil)
    let window = try XCTUnwrap(settings.window)
    let sidebar = try XCTUnwrap(
      descendants(window.contentView).first { $0 is NSTableView } as? NSTableView)
    XCTAssertEqual(sidebar.numberOfRows, 3)
    sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
    let checkbox = try XCTUnwrap(
      descendants(window.contentView).first { $0.accessibilityLabel() == "Use smart quotes" }
        as? NSButton)
    XCTAssertEqual(checkbox.state, .off)
    checkbox.performClick(nil)
    XCTAssertTrue(preferences.smartQuotes)
    XCTAssertEqual(checkbox.state, .on)
    preferences.setSmartQuotes(false)
    XCTAssertEqual(checkbox.state, .off)
    XCTAssertEqual(theme.theme, .midnight)
    window.contentView?.layoutSubtreeIfNeeded()
    XCTAssertTrue(window.contentLayoutRect.contains(checkbox.convert(checkbox.bounds, to: nil)))
  }

  func testTextWidthDefaultsToEightyPersistsRejectsZeroAndNotifiesOnlyOnChanges() throws {
    let suite = "viem-editing-width-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: suite) }
    let center = NotificationCenter()
    let preferences = EVEditingPreferences(configuration: configuration, center: center)
    var changes = 0
    let observer = center.addObserver(
      forName: .viemEditingPreferencesDidChange, object: nil, queue: .main
    ) { _ in changes += 1 }
    defer { center.removeObserver(observer) }
    XCTAssertEqual(preferences.textWidth, 80)
    XCTAssertTrue(preferences.setTextWidth(72))
    XCTAssertEqual(changes, 1)
    XCTAssertTrue(preferences.setTextWidth(72))
    XCTAssertEqual(changes, 1)
    XCTAssertFalse(preferences.setTextWidth(0))
    XCTAssertEqual(preferences.textWidth, 72)
    XCTAssertEqual(changes, 1)
    XCTAssertEqual(EVEditingPreferences(configuration: EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)).textWidth, 72)
    XCTAssertFalse(preferences.smartQuotes)
  }

  func testEditingCategoryTextWidthFieldValidatesAndFollowsPreference() throws {
    let suite = "viem-editing-width-settings-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: suite) }
    let preferences = EVEditingPreferences(configuration: configuration)
    let theme = EVThemeStore(configuration: configuration)
    let settings = EVSettingsWindowController(store: theme, editingPreferences: preferences)
    defer { settings.close() }
    settings.showWindow(nil)
    let window = try XCTUnwrap(settings.window)
    let sidebar = try XCTUnwrap(
      descendants(window.contentView).first { $0 is NSTableView } as? NSTableView)
    sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
    let field = try XCTUnwrap(
      descendants(window.contentView).first { $0.accessibilityLabel() == "Text width (columns)" }
        as? NSTextField)
    XCTAssertEqual(field.stringValue, "80")
    field.stringValue = "72"
    XCTAssertTrue(field.sendAction(field.action, to: field.target))
    XCTAssertEqual(preferences.textWidth, 72)
    XCTAssertEqual(configuration.textWidth, 72)
    for invalid in ["0", "-5", "1.5", "abc", "", "4294967296"] {
      field.stringValue = invalid
      XCTAssertTrue(field.sendAction(field.action, to: field.target))
      XCTAssertEqual(preferences.textWidth, 72, invalid)
      XCTAssertEqual(field.stringValue, "72", invalid)
    }
    preferences.setTextWidth(60)
    XCTAssertEqual(field.stringValue, "60")
    XCTAssertFalse(preferences.smartQuotes)
    XCTAssertEqual(theme.theme, .midnight)
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
