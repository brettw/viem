import XCTest
import CoreText
@testable import ViemCoreTextProvider

final class EVFontVariationsTests: XCTestCase {
  func testModifiersRestoreBaseCoordinatesAndClampToFontLimits() {
    let info = EVFontVariationInfo(axes: [
      EVFontAxis(tag: "wght", name: "Weight", minimum: 300, defaultValue: 400, maximum: 700, hidden: false),
      EVFontAxis(tag: "slnt", name: "Slant", minimum: -20, defaultValue: 0, maximum: 0, hidden: false)
    ], instances: [], links: [:])
    let saved = ["wght": 450.25, "slnt": -18.0]
    let boldItalic = EVFontVariations.effective(info, saved: saved, weight: 750, bold: true, slant: 1)
    XCTAssertEqual(boldItalic["wght"], 700)
    XCTAssertEqual(boldItalic["slnt"], -18)
    XCTAssertEqual(EVFontVariations.effective(info, saved: [:], weight: 400, bold: false, slant: 1)["slnt"], -12)
    XCTAssertEqual(EVFontVariations.effective(info, saved: saved, weight: 450, bold: false, slant: 0), saved)
    XCTAssertEqual(EVFontVariations.decode(EVFontVariations.encode(saved)), saved)
  }
  func testItalicAxisAndStyleLinkedWeightUseDesignedTargets() {
    let info = EVFontVariationInfo(axes: [
      EVFontAxis(tag: "wght", name: "Weight", minimum: 100, defaultValue: 400, maximum: 900, hidden: false),
      EVFontAxis(tag: "ital", name: "Italic", minimum: 0, defaultValue: 0, maximum: 1, hidden: false)
    ], instances: [], links: ["wght": [EVFontStyleLink(from: 300, to: 550), EVFontStyleLink(from: 400, to: 650)]])
    let values = EVFontVariations.effective(info, saved: ["wght": 400, "ital": 0.5], weight: 700, bold: true, slant: 1)
    XCTAssertEqual(values["wght"], 650); XCTAssertEqual(values["ital"], 1)
    XCTAssertEqual(EVFontVariations.effective(info, saved: ["wght": 300], weight: 600, bold: true, slant: 0)["wght"], 550)
  }
}
