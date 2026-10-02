import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVSettingsKeyboardNavigationTests: XCTestCase {
  func testEditingOffersEnabledMarkdownAutodetectionAndPersistsClick() throws {
    let configuration = configuration()
    let settings = settings(configuration)
    defer { settings.close() }
    settings.showWindow(nil)
    let window = try XCTUnwrap(settings.window)
    let sidebar = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSTableView }.first)
    sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
    let checkbox = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSButton }
      .first { $0.title == "Automatically format typed Markdown" })
    XCTAssertEqual(checkbox.state, .on)
    checkbox.performClick(nil)
    XCTAssertFalse(configuration.markdownAutodetect)
    XCTAssertFalse(EVConfigurationStore(directory: configuration.directory).markdownAutodetect)
  }
  private func configuration() -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-settings-keyboard-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return EVConfigurationStore(directory: directory)
  }

  private func settings(_ configuration: EVConfigurationStore) -> EVSettingsWindowController {
    EVSettingsWindowController(store: EVThemeStore(configuration: configuration),
      editingPreferences: EVEditingPreferences(configuration: configuration),
      viewPreferences: EVViewPreferences(configuration: configuration))
  }

  func testMarginTabAndBacktabCommitValuesInDisplayedOrder() throws {
    let configuration = configuration()
    let settings = settings(configuration)
    defer { settings.close() }
    settings.showWindow(nil)
    settings.showViewCategoryForTesting()
    let window = try XCTUnwrap(settings.window)
    window.makeKeyAndOrderFront(nil)
    let fields = try ["top", "left", "bottom", "right"].map { side in
      try field("View \(side) margin", in: window)
    }
    XCTAssertTrue(window.makeFirstResponder(fields[0]))
    for index in 0..<3 {
      let editor = try XCTUnwrap(fields[index].currentEditor() as? NSTextView)
      editor.selectAll(nil)
      editor.insertText(String(20 + index), replacementRange: editor.selectedRange())
      editor.insertTab(nil)
      XCTAssertTrue(fields[index + 1].currentEditor() === window.firstResponder)
    }
    let rightEditor = try XCTUnwrap(fields[3].currentEditor() as? NSTextView)
    rightEditor.selectAll(nil)
    rightEditor.insertText("23", replacementRange: rightEditor.selectedRange())
    for index in stride(from: 3, through: 1, by: -1) {
      let editor = try XCTUnwrap(fields[index].currentEditor() as? NSTextView)
      editor.insertBacktab(nil)
      XCTAssertTrue(fields[index - 1].currentEditor() === window.firstResponder)
    }
    XCTAssertEqual(EVConfigurationStore(directory: configuration.directory).viewMargins,
      EVViewMargins(top: 20, left: 21, bottom: 22, right: 23))
  }

  func testInvalidMarginRevertsAndTabStillAdvances() throws {
    let configuration = configuration()
    let settings = settings(configuration)
    defer { settings.close() }
    settings.showWindow(nil)
    settings.showViewCategoryForTesting()
    let window = try XCTUnwrap(settings.window)
    window.makeKeyAndOrderFront(nil)
    let top = try field("View top margin", in: window)
    let left = try field("View left margin", in: window)
    XCTAssertTrue(window.makeFirstResponder(top))
    let editor = try XCTUnwrap(top.currentEditor() as? NSTextView)
    editor.selectAll(nil)
    editor.insertText("invalid", replacementRange: editor.selectedRange())
    editor.insertTab(nil)
    XCTAssertEqual(top.doubleValue, configuration.viewMargins.top)
    XCTAssertTrue(left.currentEditor() === window.firstResponder)
  }

  func testCategoryChangeCommitsPendingEditAndRebuildsNativeKeyLoop() throws {
    let configuration = configuration()
    let settings = settings(configuration)
    defer { settings.close() }
    settings.showWindow(nil)
    settings.showViewCategoryForTesting()
    let window = try XCTUnwrap(settings.window)
    window.makeKeyAndOrderFront(nil)
    let top = try field("View top margin", in: window)
    XCTAssertTrue(window.makeFirstResponder(top))
    let editor = try XCTUnwrap(top.currentEditor() as? NSTextView)
    editor.selectAll(nil)
    editor.insertText("37", replacementRange: editor.selectedRange())
    let sidebar = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSTableView }.first)
    XCTAssertEqual(sidebar.numberOfRows, 3)
    sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
    XCTAssertEqual(configuration.viewMargins.top, 37)
    let width = try field("Text width (columns)", in: window)
    let tabstop = try field("Indentation tabstop", in: window)
    let shiftwidth = try field("Indentation shiftwidth", in: window)
    XCTAssertTrue(width.nextKeyView === tabstop)
    XCTAssertTrue(tabstop.nextKeyView === shiftwidth)
    XCTAssertTrue(window.makeFirstResponder(width))
    let widthEditor = try XCTUnwrap(width.currentEditor() as? NSTextView)
    widthEditor.selectAll(nil)
    widthEditor.insertText("96", replacementRange: widthEditor.selectedRange())
    widthEditor.insertTab(nil)
    XCTAssertTrue(tabstop.currentEditor() === window.firstResponder)
    XCTAssertEqual(configuration.textWidth, 96)
    settings.showViewCategoryForTesting()
    let newTop = try field("View top margin", in: window)
    XCTAssertFalse(newTop === top)
    XCTAssertTrue(sidebar.nextKeyView === newTop)
    XCTAssertEqual(newTop.doubleValue, 37)
  }

  private func field(_ label: String, in window: NSWindow) throws -> NSTextField {
    try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSTextField }
      .first { $0.accessibilityLabel() == label })
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
