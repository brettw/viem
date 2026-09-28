import AppKit
import CoreText
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor final class EVKerningTests: XCTestCase {
  func testPreviewZeroLetterSpacingAndImportedKerningSwitchKeepNativeKerning() throws {
    let preview = EVCoreTextStylePreviewView(frame: NSRect(x: 0, y: 0, width: 1400, height: 180))
    var values: [EVStyleProperty: EVStyleValue] = [
      .characterFontFamilies: .stringList(["Times New Roman"]),
      .characterSize: .float(24),
    ]
    func width() throws -> CGFloat {
      preview.apply(kind: .character, effectiveValues: values)
      return try XCTUnwrap(preview.inspection().currentStyleLines.first).typographicWidth
    }
    let native = try width()
    values[.characterLetterSpacing] = .float(0)
    XCTAssertEqual(try width(), native, accuracy: 0.001)
    values[.characterOpenTypeFeatures] = .openTypeFeatures([
      EVOpenTypeFeature(tag: "kern", setting: 0)
    ])
    XCTAssertEqual(try width(), native, accuracy: 0.001)
    values[.characterLetterSpacing] = .float(2)
    XCTAssertGreaterThan(try width(), native)
  }

}
