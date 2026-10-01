import AppKit
import CoreText
import Foundation
import Testing
import CViemCore

@testable import ViemCoreTextProvider

@Suite("Bundled desktop fonts", .serialized)
@MainActor
struct EVBundledFontTests {
  @Test func relocatedResourcesExposeEveryFaceToPickerAndShaper() throws {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    let original = root.appendingPathComponent("assets/fonts")
    let parent = FileManager.default.temporaryDirectory.appendingPathComponent("Viem fonts ü \(UUID().uuidString)")
    let directory = parent.appendingPathComponent("Resources/fonts")
    try FileManager.default.createDirectory(at: directory.deletingLastPathComponent(), withIntermediateDirectories: true)
    try FileManager.default.copyItem(at: original, to: directory)
    let files = try #require(FileManager.default.enumerator(at: directory, includingPropertiesForKeys: nil))
      .compactMap { $0 as? URL }.filter { ["ttf", "ttc"].contains($0.pathExtension) }
    defer {
      for file in files { CTFontManagerUnregisterFontsForURL(file as CFURL, .process, nil) }
      EVFontCatalog.invalidate()
      try? FileManager.default.removeItem(at: parent)
    }
    #expect(files.count == 13)
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    let expectedFamilies = ["Flightline Code", "Recursive Mono Casual Static", "Recursive Mono Linear Static",
      "Recursive Sans Casual Static", "Recursive Sans Linear Static"]
    for family in expectedFamilies {
      #expect(NSFontManager.shared.availableFontFamilies.contains(family))
      #expect(EVFontCatalog.faces(for: family).count == (family == "Flightline Code" ? 12 : 16))
    }
    var count = 0
    for file in files {
      let descriptors = try #require(CTFontManagerCreateFontDescriptorsFromURL(file as CFURL) as? [CTFontDescriptor])
      for descriptor in descriptors {
        let originalFont = CTFontCreateWithFontDescriptor(descriptor, 17, nil)
        let name = CTFontCopyPostScriptName(originalFont) as String
        let face = try #require(EVFontCatalog.face(named: name))
        let font = resolveFont(families: [name], size: 17, cssWeight: CGFloat(face.weight),
          slant: face.italic ? UInt32(VIEM_FONT_SLANT_ITALIC) : UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
        #expect(CTFontCopyPostScriptName(font) as String == name)
        #expect(EVFontCatalog.weight(of: font) == face.weight)
        let url = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
        #expect(url?.standardizedFileURL == file.standardizedFileURL)
        let sample = NSAttributedString(string: "Writing 0123", attributes: [.font: font])
        let line = CTLineCreateWithAttributedString(sample)
        #expect(CTLineGetTypographicBounds(line, nil, nil, nil) > 0)
        let runs = CTLineGetGlyphRuns(line) as? [CTRun] ?? []
        #expect(!runs.isEmpty)
        for run in runs {
          let attributes = CTRunGetAttributes(run) as NSDictionary
          let rendered = try #require(attributes[kCTFontAttributeName]) as! CTFont
          #expect(CTFontCopyPostScriptName(rendered) as String == name)
        }
        count += 1
      }
    }
    #expect(count == 76)
  }

  @Test func missingOrInvalidFontsLeaveSystemFallbackAvailable() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("Viem invalid fonts \(UUID().uuidString)")
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    try Data("invalid font".utf8).write(to: directory.appendingPathComponent("broken.ttf"))
    #expect(EVFontCatalog.registerBundledFonts(in: directory).count == 1)
    #expect(!EVFontCatalog.faces(for: "system-ui").isEmpty)
  }
}
