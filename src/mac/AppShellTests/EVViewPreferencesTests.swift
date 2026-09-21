import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVViewPreferencesTests: XCTestCase {
  private func configuration() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-view-settings-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory)
  }

  func testMarginsPersistIndependentlyOfThemeAndRejectInvalidValues() throws {
    let config = configuration()
    let center = NotificationCenter()
    let preferences = EVViewPreferences(configuration: config, center: center)
    XCTAssertEqual(preferences.margins, EVViewMargins(top: 10, left: 10, bottom: 10, right: 10))
    var notifications = 0
    let observer = center.addObserver(forName: .viemViewPreferencesDidChange, object: nil, queue: .main) { _ in notifications += 1 }
    defer { center.removeObserver(observer) }
    let margins = EVViewMargins(top: 12, left: 24, bottom: 36, right: 48)
    XCTAssertTrue(preferences.setMargins(margins))
    XCTAssertEqual(notifications, 1)
    XCTAssertTrue(preferences.setMargins(margins))
    for value in [-1, Double.nan, Double.infinity, 1001] {
      XCTAssertFalse(preferences.setMargins(EVViewMargins(left: value)))
    }
    XCTAssertEqual(notifications, 1)
    try config.setTheme(.midnight)
    let reopened = EVConfigurationStore(directory: config.directory)
    XCTAssertEqual(reopened.viewMargins, margins)
    XCTAssertEqual(reopened.theme, .midnight)
    let json = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: config.directory.appendingPathComponent("config.json"))) as? [String: Any])
    XCTAssertNil((json["theme"] as? [String: Any])?["padding"])
    XCTAssertNotNil((json["view"] as? [String: Any])?["margins"])
  }

  func testViewCategoryReplacesDocumentsAndEditsEveryMargin() throws {
    let config = configuration()
    let preferences = EVViewPreferences(configuration: config)
    let settings = EVSettingsWindowController(store: EVThemeStore(configuration: config),
      editingPreferences: EVEditingPreferences(configuration: config), viewPreferences: preferences)
    defer { settings.close() }
    settings.showWindow(nil)
    settings.showViewCategoryForTesting()
    let window = try XCTUnwrap(settings.window)
    window.contentView?.layoutSubtreeIfNeeded()
    let views = descendants(window.contentView)
    let sidebar = try XCTUnwrap(views.compactMap { $0 as? NSTableView }.first)
    let titles = (0..<sidebar.numberOfRows).flatMap { row in
      descendants(settings.tableView(sidebar, viewFor: sidebar.tableColumns.first, row: row))
        .compactMap { ($0 as? NSTextField)?.stringValue }
    }
    XCTAssertEqual(titles, ["View", "Theme", "Editing"])
    let fields = views.compactMap { $0 as? NSTextField }
    for (side, value) in [("top", 15.0), ("left", 25), ("bottom", 35), ("right", 45)] {
      let field = try XCTUnwrap(fields.first { $0.accessibilityLabel() == "View \(side) margin" })
      field.doubleValue = value
      XCTAssertTrue(field.sendAction(try XCTUnwrap(field.action), to: field.target))
    }
    XCTAssertEqual(preferences.margins, EVViewMargins(top: 15, left: 25, bottom: 35, right: 45))
    let top = try XCTUnwrap(fields.first { $0.accessibilityLabel() == "View top margin" })
    top.stringValue = "invalid"
    top.sendAction(try XCTUnwrap(top.action), to: top.target)
    XCTAssertEqual(top.doubleValue, 15)
    XCTAssertFalse(fields.contains { $0.accessibilityLabel() == "Status font size" })
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
