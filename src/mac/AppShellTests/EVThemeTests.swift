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
    XCTAssertNotNil(fields.first { $0.accessibilityLabel() == "Status font size" })
    XCTAssertFalse(fields.contains { $0.accessibilityLabel()?.contains("margin") == true })
    let categories = try XCTUnwrap(
      descendants(window.contentView).compactMap { $0 as? NSTableView }.first)
    XCTAssertGreaterThan(
      categories.convert(categories.rect(ofRow: 0), to: nil).midY,
      window.contentLayoutRect.height / 2)
  }

  func testStatusFontSizeArrowsCommitPendingTextAndTrackThemeChanges() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-theme-stepper-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let configuration = EVConfigurationStore(directory: directory)
    let store = EVThemeStore(configuration: configuration)
    var theme = store.theme
    theme.statusFontSize = 14.5
    store.update(theme)
    let settings = EVSettingsWindowController(store: store)
    defer { settings.close() }
    settings.showWindow(nil)
    let window = try XCTUnwrap(settings.window)
    window.makeKeyAndOrderFront(nil)
    window.contentView?.layoutSubtreeIfNeeded()
    let field = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSTextField }
      .first { $0.accessibilityLabel() == "Status font size" })
    let stepper = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSStepper }
      .first { $0.accessibilityLabel() == "Adjust status font size" })
    XCTAssertTrue(stepper.superview === field.superview)
    let siblings = try XCTUnwrap(field.superview as? NSStackView).arrangedSubviews
    XCTAssertEqual(try XCTUnwrap(siblings.firstIndex(of: stepper)), try XCTUnwrap(siblings.firstIndex(of: field)) + 1)
    XCTAssertEqual(stepper.increment, 1)
    XCTAssertEqual(stepper.minValue, 8)
    XCTAssertEqual(stepper.maxValue, 32)
    XCTAssertFalse(stepper.valueWraps)

    func clickArrow(up: Bool) throws {
      stepper.scrollToVisible(stepper.bounds)
      let top = up != stepper.isFlipped
      let location = stepper.convert(NSPoint(x: stepper.bounds.midX,
        y: stepper.bounds.height * (top ? 0.75 : 0.25)), to: nil)
      func event(_ type: NSEvent.EventType) throws -> NSEvent {
        try XCTUnwrap(NSEvent.mouseEvent(with: type, location: location, modifierFlags: [],
          timestamp: 0, windowNumber: window.windowNumber, context: nil,
          eventNumber: 1, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0))
      }
      NSApplication.shared.postEvent(try event(.leftMouseUp), atStart: true)
      window.sendEvent(try event(.leftMouseDown))
    }

    try clickArrow(up: true)
    XCTAssertEqual(store.theme.statusFontSize, 15.5)
    XCTAssertEqual(field.doubleValue, 15.5)
    try clickArrow(up: false)
    XCTAssertEqual(store.theme.statusFontSize, 14.5)
    XCTAssertTrue(window.makeFirstResponder(field))
    let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
    editor.insertText("20.5", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    try clickArrow(up: true)
    XCTAssertEqual(store.theme.statusFontSize, 21.5)
    theme = store.theme
    theme.statusFontSize = 32
    store.update(theme)
    XCTAssertEqual(stepper.doubleValue, 32)
    try clickArrow(up: true)
    XCTAssertEqual(store.theme.statusFontSize, 32)
    XCTAssertEqual(EVThemeStore(configuration: EVConfigurationStore(directory: directory)).theme.statusFontSize, 32)
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
