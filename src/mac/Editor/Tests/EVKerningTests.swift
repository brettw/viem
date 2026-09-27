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

  func testRichClipboardPreservesTrackingWithoutDisablingKerning() throws {
    for spacing in [0.0, 2.0, -0.5] {
      let source = "{\\rtf1\\ansi{\\fonttbl{\\f0 Times New Roman;}}\\f0\\fs48\\expndtw\(Int(spacing * 20)) AV}"
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: EVDocument.rtfType)
      let fragment = try backend.clipboardFragment(in: 0..<2, snapshot: backend.formattedSnapshot())
      let attributed = try fragment.attributedText()
      let font = try XCTUnwrap(attributed.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
      let reference = NSAttributedString(
        string: "AV", attributes: [.font: font, .tracking: spacing])
      XCTAssertEqual(
        CTLineGetTypographicBounds(CTLineCreateWithAttributedString(attributed), nil, nil, nil),
        CTLineGetTypographicBounds(CTLineCreateWithAttributedString(reference), nil, nil, nil),
        accuracy: 0.001)
      let rtf = try attributed.data(
        from: NSRange(location: 0, length: attributed.length),
        documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
      XCTAssertFalse(String(decoding: rtf, as: UTF8.self).contains("\\kerning0"))
      let reopened = try NSAttributedString(
        data: rtf,
        options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
      if spacing == 0 {
        XCTAssertNil(attributed.attribute(.kern, at: 0, effectiveRange: nil))
        XCTAssertNil(reopened.attribute(.kern, at: 0, effectiveRange: nil))
      } else {
        XCTAssertEqual(
          try XCTUnwrap(reopened.attribute(.kern, at: 0, effectiveRange: nil) as? NSNumber)
            .doubleValue,
          spacing, accuracy: 0.001)
      }
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8))
    }
  }
}
