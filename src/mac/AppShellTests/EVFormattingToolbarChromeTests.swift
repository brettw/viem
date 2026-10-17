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
    var isLiteralMarkdown = false
    let formattingToolbarView = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 42))
    var refreshes = 0
    var previewRequests = 0
    func refreshFormattingToolbar() { refreshes += 1 }
    func showMarkdownPreview() {
      previewRequests += 1
      formattingToolbarFormat = .markdown
      isLiteralMarkdown = false
    }
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
    let second = Surface()
    chrome.synchronize(surface: second)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 1, "Markdown remembers hidden")
    XCTAssertNil(first.formattingToolbarView.window)
    try config.setShowFormattingToolbar(true, for: .markdown)
    chrome.synchronize(surface: second)
    XCTAssertTrue(second.formattingToolbarView.window === window)
    XCTAssertNil(first.formattingToolbarView.window)
    // A preference changed in another window updates this one too.
    try config.setShowFormattingToolbar(false, for: .markdown)
    XCTAssertEqual(window.titlebarAccessoryViewControllers.count, 1)
    XCTAssertEqual(chrome.toggleButton.state, .off)
  }

  func testLiteralMarkdownOffersPreviewInTheToolbarTogglePlace() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let config = EVConfigurationStore(directory: directory)
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
      styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    defer { window.orderOut(nil) }
    let chrome = EVFormattingToolbarChrome(window: window, configuration: config)
    let surface = Surface()
    chrome.synchronize(surface: surface)
    XCTAssertFalse(chrome.toggleButton.isHidden)
    XCTAssertTrue(chrome.previewButton.isHidden, "Preview shows only the toolbar toggle")
    XCTAssertEqual(chrome.previewButton.accessibilityLabel(), "Markdown Preview")
    XCTAssertEqual(chrome.previewButton.toolTip, "Markdown Preview")
    XCTAssertTrue(chrome.previewButton.superview === chrome.toggleButton.superview)
    XCTAssertEqual(chrome.previewButton.frame, chrome.toggleButton.frame)
    for visible in [true, false] {
      try config.setShowFormattingToolbar(visible, for: .markdown)
      surface.formattingToolbarFormat = .code
      surface.isLiteralMarkdown = true
      chrome.synchronize(surface: surface)
      XCTAssertEqual(window.titlebarAccessoryViewControllers.map(\.layoutAttribute), [.right],
        "literal Markdown has no toolbar whatever its preference")
      XCTAssertNil(surface.formattingToolbarView.window)
      XCTAssertTrue(chrome.toggleButton.isHidden)
      XCTAssertFalse(chrome.previewButton.isHidden)
      // Hidden toolbar toggles cannot change the Markdown preference.
      chrome.toggleToolbar(nil)
      XCTAssertEqual(config.showFormattingToolbar(for: .markdown), visible)
      let requests = surface.previewRequests
      chrome.previewButton.performClick(nil)
      XCTAssertEqual(surface.previewRequests, requests + 1)
      XCTAssertFalse(chrome.toggleButton.isHidden)
      XCTAssertTrue(chrome.previewButton.isHidden)
      XCTAssertEqual(chrome.toggleButton.state, visible ? .on : .off,
        "Preview restores the remembered toolbar visibility")
      XCTAssertEqual(surface.formattingToolbarView.window === window, visible)
      XCTAssertEqual(window.titlebarAccessoryViewControllers.count, visible ? 2 : 1)
    }
    chrome.previewButton.performClick(nil)
    XCTAssertEqual(surface.previewRequests, 2, "Preview cannot be requested again once it is on")
  }

  func testSettingsMergeAndRejectNonBooleanVisibility() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let first = EVConfigurationStore(directory: directory)
    let second = EVConfigurationStore(directory: directory)
    try first.setShowFormattingToolbar(false, for: .markdown)
    try second.setShowFormattingToolbar(false, for: .plainText)
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertFalse(reopened.showFormattingToolbar(for: .markdown))
    XCTAssertFalse(reopened.showFormattingToolbar(for: .plainText))
    XCTAssertTrue(reopened.showFormattingToolbar(for: .code))
    let file = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"formattingToolbar":{"markdown":1}}"#.utf8).write(to: file)
    XCTAssertNotNil(EVConfigurationStore(directory: directory).lastError)
  }
}
