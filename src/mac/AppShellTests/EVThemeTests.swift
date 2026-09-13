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
    theme.statusFontFamily = "Georgia"
    theme.statusFontSize = 16
    store.update(theme)
    XCTAssertEqual(notifications, 1)
    XCTAssertEqual(EVThemeStore(configuration: EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)).theme, theme)
    XCTAssertEqual(store.theme.statusFont.pointSize, 16)
    let generation = store.generation
    store.update(theme)
    var invalid = theme
    invalid.foreground.red = .nan
    store.update(invalid)
    invalid = theme
    invalid.statusFontSize = 0
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
    defaults.set(Data("invalid json".utf8), forKey: "EVApplicationTheme.v1")
    let store = EVThemeStore(configuration: configuration)
    XCTAssertEqual(store.theme, .paper)
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
    XCTAssertNotNil(fields.first { $0.accessibilityLabel() == "Status font size" })
    XCTAssertFalse(fields.contains { $0.accessibilityLabel()?.contains("margin") == true })
    let categories = try XCTUnwrap(
      descendants(window.contentView).compactMap { $0 as? NSTableView }.first)
    XCTAssertGreaterThan(
      categories.convert(categories.rect(ofRow: 0), to: nil).midY,
      window.contentLayoutRect.height / 2)
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
