import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVIndentationPreferencesTests: XCTestCase {
  private func fixture() -> (URL, EVConfigurationStore) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-indentation-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return (directory, EVConfigurationStore(directory: directory))
  }

  func testDefaultsAndIndependentWhitespaceSettingsRoundTrip() throws {
    let (directory, store) = fixture()
    XCTAssertEqual(store.indentation, EVIndentationOptions())
    XCTAssertEqual(store.indentation.tabstop, 2)
    XCTAssertEqual(store.indentation.shiftwidth, 2)
    XCTAssertEqual(store.indentation.softtabstop, 2)
    XCTAssertTrue(store.indentation.autoindent && store.indentation.expandtab && store.indentation.smarttab)
    XCTAssertTrue(store.indentation.continueCommentsOnEnter && store.indentation.continueCommentsOnOpenLine)
    XCTAssertEqual(store.whitespacePresentation.codeWhitespace, .paragraphEn)
    XCTAssertEqual(store.whitespacePresentation.otherWhitespace, .spaces)
    XCTAssertEqual(store.whitespacePresentation.codeWrappedLineIndent, 4)
    XCTAssertTrue(store.whitespacePresentation.visibleWhitespace.enabled)
    XCTAssertEqual(store.whitespacePresentation.visibleWhitespace.listchars, "tab:>-,trail:*,extends:>,precedes:<")
    XCTAssertEqual(store.whitespacePresentation.visibleWhitespace.style.foreground?.blue ?? 0, 139 / 255, accuracy: 0.000001)
    var indentation = store.indentation
    indentation.tabstop = 8
    indentation.shiftwidth = 0
    indentation.softtabstop = -1
    indentation.autoindent = false
    try store.setIndentation(indentation)
    XCTAssertEqual(store.whitespacePresentation.codeWrappedLineIndent, 4)
    var whitespace = store.whitespacePresentation
    whitespace.otherWhitespace = .paragraphEn
    whitespace.codeWrappedLineIndent = 6
    whitespace.visibleWhitespace.listchars = "tab:>-,leadtab:>.,space:·,eol:$"
    try store.setWhitespacePresentation(whitespace)
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertEqual(reopened.indentation, indentation)
    XCTAssertEqual(reopened.whitespacePresentation, whitespace)
    let json = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("config.json"))) as? [String: Any])
    let editing = try XCTUnwrap(json["editing"] as? [String: Any])
    XCTAssertNotNil((editing["indentation"] as? [String: Any])?["continueCommentsOnOpenLine"])
    XCTAssertEqual((editing["whitespacePresentation"] as? [String: Any])?["codeWrappedLineIndent"] as? Int, 6)
  }

  func testPartialDefaultsRejectNullAndInvalidConfigurationWithoutOverwriting() throws {
    for fragment in [
      #""indentation":{"tabstop":0}"#, #""indentation":{"tabstop":1025}"#,
      #""indentation":{"shiftwidth":-1}"#, #""indentation":{"softtabstop":-2}"#,
      #""indentation":{"autoindent":null}"#, #""indentation":{"expandtab":1}"#,
      #""whitespacePresentation":{"codeWhitespace":"pixels"}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":-1}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":1025}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":1.5}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":null}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":true}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":"4"}"#,
      #""whitespacePresentation":{"codeWrappedLineIndent":18446744073709551616}"#,
      #""whitespacePresentation":{"visibleWhitespace":{"listchars":"tab:>"}}"#,
      #""whitespacePresentation":{"visibleWhitespace":{"style":{"size":0}}}"#,
    ] {
      let (directory, _) = fixture()
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      let file = directory.appendingPathComponent("config.json")
      let before = Data("{\"version\":1,\"editing\":{\(fragment)}}".utf8)
      try before.write(to: file)
      let store = EVConfigurationStore(directory: directory)
      XCTAssertNotNil(store.lastError, fragment)
      XCTAssertThrowsError(try store.setIndentation(EVIndentationOptions()), fragment)
      XCTAssertEqual(try Data(contentsOf: file), before, fragment)
    }
    let (directory, _) = fixture()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    try Data(#"{"version":1,"editing":{"indentation":{"shiftwidth":0},"whitespacePresentation":{"visibleWhitespace":{"enabled":false}}}}"#.utf8).write(to: directory.appendingPathComponent("config.json"))
    let store = EVConfigurationStore(directory: directory)
    XCTAssertNil(store.lastError)
    XCTAssertEqual(store.indentation.shiftwidth, 0)
    XCTAssertEqual(store.indentation.tabstop, 2)
    XCTAssertEqual(store.whitespacePresentation.codeWhitespace, .paragraphEn)
    XCTAssertEqual(store.whitespacePresentation.codeWrappedLineIndent, 4)
    XCTAssertFalse(store.whitespacePresentation.visibleWhitespace.enabled)
  }

  func testWrappedLineIndentUpdatesLiveAndRejectsInvalidValuesAtomically() throws {
    let (directory, store) = fixture()
    let center = NotificationCenter()
    let preferences = EVEditingPreferences(configuration: store, center: center)
    let other = EVEditingPreferences(configuration: EVConfigurationStore(directory: directory))
    var changes = 0
    let observer = center.addObserver(forName: .viemEditingPreferencesDidChange, object: nil, queue: .main) { _ in changes += 1 }
    defer { center.removeObserver(observer) }
    var options = preferences.whitespacePresentation
    for (index, value) in [0, 1024].enumerated() {
      options.codeWrappedLineIndent = value
      XCTAssertTrue(preferences.setWhitespacePresentation(options))
      XCTAssertEqual(other.whitespacePresentation.codeWrappedLineIndent, value)
      XCTAssertEqual(EVConfigurationStore(directory: directory).whitespacePresentation.codeWrappedLineIndent, value)
      XCTAssertEqual(changes, index + 1)
      XCTAssertTrue(preferences.setWhitespacePresentation(options))
      XCTAssertEqual(changes, index + 1)
    }
    let file = directory.appendingPathComponent("config.json")
    let before = try Data(contentsOf: file)
    for value in [-1, 1025, Int.max] {
      options.codeWrappedLineIndent = value
      XCTAssertFalse(preferences.setWhitespacePresentation(options))
      XCTAssertEqual(preferences.whitespacePresentation.codeWrappedLineIndent, 1024)
      XCTAssertEqual(other.whitespacePresentation.codeWrappedLineIndent, 1024)
      XCTAssertEqual(changes, 2)
      XCTAssertEqual(try Data(contentsOf: file), before)
    }
  }

  func testKnownStylePropertiesCanBeClearedWhileUnknownNestedKeysSurvive() throws {
    let (directory, _) = fixture()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let file = directory.appendingPathComponent("config.json")
    try Data(#"{"version":1,"editing":{"future":7,"indentation":{"futureIndent":8},"whitespacePresentation":{"futureWidth":9,"visibleWhitespace":{"futureMarker":10,"style":{"bold":true,"futureStyle":11}}}}}"#.utf8).write(to: file)
    let store = EVConfigurationStore(directory: directory)
    var whitespace = store.whitespacePresentation
    whitespace.visibleWhitespace.style.bold = nil
    try store.setWhitespacePresentation(whitespace)
    var indentation = store.indentation
    indentation.tabstop = 4
    try store.setIndentation(indentation)
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    let editing = try XCTUnwrap(object["editing"] as? [String: Any])
    let presentation = try XCTUnwrap(editing["whitespacePresentation"] as? [String: Any])
    let visible = try XCTUnwrap(presentation["visibleWhitespace"] as? [String: Any])
    let style = try XCTUnwrap(visible["style"] as? [String: Any])
    XCTAssertEqual(editing["future"] as? Int, 7)
    XCTAssertEqual((editing["indentation"] as? [String: Any])?["futureIndent"] as? Int, 8)
    XCTAssertEqual(presentation["futureWidth"] as? Int, 9)
    XCTAssertEqual(visible["futureMarker"] as? Int, 10)
    XCTAssertEqual(style["futureStyle"] as? Int, 11)
    XCTAssertTrue(style["bold"] is NSNull)
    XCTAssertNil(EVConfigurationStore(directory: directory).whitespacePresentation.visibleWhitespace.style.bold)
  }

  func testSettingsOwnersStaySynchronizedWithExactlyOneNotificationAndAtomicRejection() throws {
    let (directory, store) = fixture()
    let center = NotificationCenter()
    let preferences = EVEditingPreferences(configuration: store, center: center)
    let other = EVEditingPreferences(configuration: EVConfigurationStore(directory: directory))
    var changes = 0
    let observer = center.addObserver(forName: .viemEditingPreferencesDidChange, object: nil, queue: .main) { _ in changes += 1 }
    defer { center.removeObserver(observer) }
    var options = preferences.indentation
    options.shiftwidth = 4
    XCTAssertTrue(preferences.setIndentation(options))
    XCTAssertEqual(changes, 1)
    XCTAssertEqual(other.indentation.shiftwidth, 4)
    XCTAssertTrue(preferences.setIndentation(options))
    XCTAssertEqual(changes, 1)
    options.tabstop = 0
    XCTAssertFalse(preferences.setIndentation(options))
    XCTAssertEqual(changes, 1)
    XCTAssertEqual(preferences.indentation.tabstop, 2)
    var whitespace = store.whitespacePresentation
    whitespace.visibleWhitespace.style.bold = true
    try store.setWhitespacePresentation(whitespace)
    XCTAssertEqual(changes, 2)
    XCTAssertEqual(other.whitespacePresentation.visibleWhitespace.style.bold, true)
    let file = directory.appendingPathComponent("config.json")
    let before = try Data(contentsOf: file)
    let oldValidator = EVEditingPreferences.validateWhitespacePresentation
    EVEditingPreferences.validateWhitespacePresentation = { _ in "Portable validator rejected marker width" }
    defer { EVEditingPreferences.validateWhitespacePresentation = oldValidator }
    whitespace.visibleWhitespace.listchars = "eol:界"
    XCTAssertFalse(preferences.setWhitespacePresentation(whitespace))
    XCTAssertEqual(changes, 2)
    XCTAssertEqual(try Data(contentsOf: file), before)
  }

  func testEditingControlsValidateAndExposeAllTwelveEntriesAndStyleAction() throws {
    let (_, store) = fixture()
    let preferences = EVEditingPreferences(configuration: store)
    let settings = EVSettingsWindowController(store: EVThemeStore(configuration: store), editingPreferences: preferences)
    defer { settings.close() }
    let window = try XCTUnwrap(settings.window)
    let sidebar = try XCTUnwrap(descendants(window.contentView).first { $0 is NSTableView } as? NSTableView)
    sidebar.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
    func field(_ label: String) throws -> NSTextField {
      try XCTUnwrap(descendants(window.contentView).first { $0.accessibilityLabel() == label } as? NSTextField)
    }
    let tabstop = try field("Indentation tabstop")
    for input in ["0", "-1", "1.5", "1025", "abc"] {
      tabstop.stringValue = input
      tabstop.sendAction(tabstop.action, to: tabstop.target)
      XCTAssertEqual(tabstop.stringValue, "2")
    }
    let shift = try field("Indentation shiftwidth")
    shift.stringValue = "0"
    shift.sendAction(shift.action, to: shift.target)
    XCTAssertEqual(preferences.indentation.shiftwidth, 0)
    let wrappedIndent = try field("Wrapped line indent (Code)")
    XCTAssertEqual(wrappedIndent.stringValue, "4")
    for input in ["-1", "1.5", "1025", "abc", "", "18446744073709551616"] {
      wrappedIndent.stringValue = input
      wrappedIndent.sendAction(wrappedIndent.action, to: wrappedIndent.target)
      XCTAssertEqual(wrappedIndent.stringValue, "4")
      XCTAssertEqual(preferences.whitespacePresentation.codeWrappedLineIndent, 4)
    }
    for value in [0, 1024] {
      wrappedIndent.stringValue = String(value)
      wrappedIndent.sendAction(wrappedIndent.action, to: wrappedIndent.target)
      XCTAssertEqual(preferences.whitespacePresentation.codeWrappedLineIndent, value)
    }
    for name in EVListcharsSettings.names { _ = try field("Visible whitespace \(name)") }
    let trail = try field("Visible whitespace trail")
    XCTAssertEqual(trail.stringValue, "*")
    trail.stringValue = ""
    trail.sendAction(trail.action, to: trail.target)
    XCTAssertNil(EVListcharsSettings.entries(preferences.whitespacePresentation.visibleWhitespace.listchars)["trail"])
    let eol = try field("Visible whitespace eol")
    eol.stringValue = ","
    eol.sendAction(eol.action, to: eol.target)
    XCTAssertEqual(eol.stringValue, ",")
    XCTAssertEqual(EVListcharsSettings.entries(preferences.whitespacePresentation.visibleWhitespace.listchars)["eol"], "\\x2c")
    eol.stringValue = #"\u00b7"#
    eol.sendAction(eol.action, to: eol.target)
    XCTAssertEqual(eol.stringValue, "·")
    let popup = try XCTUnwrap(descendants(window.contentView).first { $0.accessibilityLabel() == "Other formats whitespace width" } as? NSPopUpButton)
    popup.selectItem(at: 1)
    popup.sendAction(popup.action, to: popup.target)
    XCTAssertEqual(preferences.whitespacePresentation.otherWhitespace, .paragraphEn)
    XCTAssertEqual(preferences.whitespacePresentation.codeWhitespace, .paragraphEn)
    var edited: EVConfigurationStore?
    let oldAction = EVEditingPreferences.editVisibleWhitespaceStyle
    EVEditingPreferences.editVisibleWhitespaceStyle = { edited = $0 }
    defer { EVEditingPreferences.editVisibleWhitespaceStyle = oldAction }
    let button = try XCTUnwrap(descendants(window.contentView).compactMap { $0 as? NSButton }.first { $0.title == "Edit Style…" })
    button.performClick(nil)
    XCTAssertTrue(edited === store)
  }

  func testListcharsEscapesAndLastEntryWins() {
    XCTAssertNil(EVListcharsSettings.validationError(#"space:\x2c,tab:>-^,space:\u00b7"#))
    XCTAssertNil(EVListcharsSettings.validationError(""))
    XCTAssertNil(EVListcharsSettings.validationError("space:x,"))
    XCTAssertNil(EVListcharsSettings.validationError("space:\\"))
    XCTAssertNil(EVListcharsSettings.validationError("tab:\\-"))
    XCTAssertNil(EVListcharsSettings.validationError("space::"))
    XCTAssertNotNil(EVListcharsSettings.validationError("space:x,,"))
    XCTAssertEqual(EVListcharsSettings.entries("space:.,space:*")["space"], "*")
    XCTAssertNotNil(EVListcharsSettings.validationError(#"eol:\uD800"#))
    XCTAssertNotNil(EVListcharsSettings.validationError("eol:\n"))
  }

  private func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
