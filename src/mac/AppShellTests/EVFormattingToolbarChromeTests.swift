import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVFormattingToolbarChromeTests: XCTestCase {
  private final class Surface: EVEditorSurface, EVFormattingToolbarProviding {
    let viewController = NSViewController()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    var formattingToolbarFormat: EVSourceFormat = .markdown
    let formattingToolbarView = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 42))
    var refreshes = 0
    func refreshFormattingToolbar() { refreshes += 1 }
    func perform(menuCommand: EVMenuCommand, sender: Any?) {}
    func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
  }

  func testVisibilityFollowsFormatPersistsAndDoesNotLeakAcrossPanes() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let config = EVConfigurationStore(directory: directory)
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
      styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    defer { window.orderOut(nil) }
    let chrome = EVFormattingToolbarChrome(window: window, configuration: config)
    let first = Surface()
    chrome.synchronize(surface: first)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 2)
    XCTAssertEqual(chrome.toggleButton.state, .on)
    XCTAssertTrue(first.formattingToolbarView.window === window)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.first?.layoutAttribute, .right)
    chrome.toggleToolbar(nil)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 1)
    XCTAssertNil(first.formattingToolbarView.window)
    XCTAssertFalse(config.showFormattingToolbar(for: .markdown))
    XCTAssertFalse(EVConfigurationStore(directory: directory).showFormattingToolbar(for: .markdown))
    for format in [EVSourceFormat.code, .plainText] {
      first.formattingToolbarFormat = format
      chrome.synchronize(surface: first)
      XCTAssertTrue(window.titlebarAccessoryViewControllers.isEmpty)
    }
    for format in [EVSourceFormat.html, .htmlSource, .markdownSource, .rtf] {
      first.formattingToolbarFormat = format
      chrome.synchronize(surface: first)
      XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 2)
    }
    let second = Surface()
    chrome.synchronize(surface: second)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 1, "Markdown remembers hidden")
    XCTAssertNil(first.formattingToolbarView.window)
    second.formattingToolbarFormat = .html
    chrome.synchronize(surface: second)
    XCTAssertTrue(second.formattingToolbarView.window === window)
    XCTAssertNil(first.formattingToolbarView.window)
    // A preference changed in another window updates this one too.
    try config.setShowFormattingToolbar(false, for: .html)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 1)
    XCTAssertEqual(chrome.toggleButton.state, .off)
  }

  func testSettingsMergeAndRejectNonBooleanVisibility() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let first = EVConfigurationStore(directory: directory)
    let second = EVConfigurationStore(directory: directory)
    try first.setShowFormattingToolbar(false, for: .markdown)
    try second.setShowFormattingToolbar(false, for: .html)
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertFalse(reopened.showFormattingToolbar(for: .markdown))
    XCTAssertFalse(reopened.showFormattingToolbar(for: .html))
    XCTAssertTrue(reopened.showFormattingToolbar(for: .markdownSource))
    let file = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"formattingToolbar":{"markdown":1}}"#.utf8).write(to: file)
    XCTAssertNotNil(EVConfigurationStore(directory: directory).lastError)
  }
}
