import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVScriptApplicationOptionsTests: XCTestCase {
  func testMappedOptionsValidateBeforeChangingEffectiveState() throws {
    let options = EVScriptApplicationOptions()
    try options.apply("timeoutlen", value: "25")
    try options.apply("timeout", value: "0")
    try options.apply("laststatus", value: "0")
    try options.apply("scrolloff", value: "5")
    XCTAssertEqual(options.timeoutLength, 25)
    XCTAssertFalse(options.timeout)
    XCTAssertEqual(options.showStatusBar, false)
    XCTAssertEqual(options.scrolloff, 5)
    XCTAssertThrowsError(try options.apply("timeoutlen", value: "-1"))
    XCTAssertEqual(options.timeoutLength, 25)
    try options.apply("breakindentopt", value: "shift:8")
    XCTAssertEqual(options.continuationIndent, 8)
    XCTAssertThrowsError(try options.apply("breakindentopt", value: "min:20"))
    XCTAssertEqual(options.continuationIndent, 8)
    try options.apply("breakindent", value: "0")
    XCTAssertEqual(options.continuationIndent, 0)
  }

  func testBellSuppressionCombinesAllThreeOptions() throws {
    let options = EVScriptApplicationOptions()
    XCTAssertTrue(options.audibleBell)
    try options.apply("visualbell", value: "1")
    XCTAssertFalse(options.audibleBell)
    try options.apply("visualbell", value: "0")
    try options.apply("belloff", value: "all")
    XCTAssertFalse(options.audibleBell)
    try options.apply("belloff", value: "")
    try options.apply("errorbells", value: "0")
    XCTAssertFalse(options.audibleBell)
  }

  func testFontAndSpacingChangeOnlyInMemoryAndRejectUnsupportedAttributes() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-script-font-\(UUID())")
    defer { try? FileManager.default.removeItem(at: directory) }
    let config = EVConfigurationStore(directory: directory)
    let configURL = directory.appendingPathComponent("config.json")
    let beforeConfig = try? Data(contentsOf: configURL)
    let themeURL = config.selectedThemeURL
    let beforeTheme = try themeURL.map { try Data(contentsOf: $0) }
    try config.applyScriptCodeOption("guifont", value: "Menlo:h18:b:i")
    try config.applyScriptCodeOption("linespace", value: "4")
    let styles = try XCTUnwrap((config.activeTheme["styles"] as? [String: Any])?["code"] as? [String: Any])
    let blocks = try XCTUnwrap(styles["block_styles"] as? [[String: Any]])
    let paragraph = try XCTUnwrap(blocks.first { $0["id"] as? String == "Paragraph" })
    let character = try XCTUnwrap(paragraph["character"] as? [String: Any])
    XCTAssertEqual(character["font_families"] as? [String], ["Menlo"])
    XCTAssertEqual(character["size"] as? Double, 18)
    XCTAssertEqual(character["slant"] as? String, "Italic")
    let accepted = try JSONSerialization.data(withJSONObject: config.activeTheme, options: .sortedKeys)
    XCTAssertThrowsError(try config.applyScriptCodeOption("guifont", value: "Menlo:h18:w20"))
    XCTAssertEqual(try JSONSerialization.data(withJSONObject: config.activeTheme, options: .sortedKeys), accepted)
    XCTAssertEqual(try? Data(contentsOf: configURL), beforeConfig)
    XCTAssertEqual(try themeURL.map { try Data(contentsOf: $0) }, beforeTheme)
  }
}
