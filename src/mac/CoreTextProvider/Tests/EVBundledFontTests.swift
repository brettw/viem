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
    #expect(files.count == 3)
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    #expect(files.contains { $0.lastPathComponent == "Recursive_VF_1.085.ttf" })
    #expect(!files.contains { $0.lastPathComponent == "recursive-static-TTFs.ttc" })
    for family in ["Flightline Code", "Recursive"] {
      #expect(NSFontManager.shared.availableFontFamilies.contains(family))
      #expect(!EVFontCatalog.faces(for: family).isEmpty)
    }
    #expect(EVFontCatalog.faces(for: "Flightline Code").count == 12)
    #expect(Set(files.filter { $0.deletingLastPathComponent().lastPathComponent == "flightline" }
      .map(\.lastPathComponent)) == ["FlightlineCode-Regular-VF.ttf", "FlightlineCode-Italic-VF.ttf"])
    for face in EVFontCatalog.faces(for: "Flightline Code") {
      let flightline = EVFontVariations.info(for: face.postScriptName)
      #expect(flightline.axes.count == 1)
      #expect(flightline.axes.first?.tag == "wght")
      #expect(flightline.axes.first?.minimum == 200)
      #expect(flightline.axes.first?.defaultValue == 400)
      #expect(flightline.axes.first?.maximum == 700)
      #expect(flightline.instances.filter { $0.name != "Default" }.count == 6)
    }
    for (name, weight, italic) in [
      ("Thin", 100, false), ("ThinItalic", 100, true),
      ("ExtraLight", 200, false), ("ExtLtIta", 200, true),
      ("Light", 300, false), ("LightItalic", 300, true),
      ("Regular", 400, false), ("Italic", 400, true),
      ("Medium", 500, false), ("MediumItalic", 500, true),
      ("Bold", 700, false), ("BoldItalic", 700, true),
    ] {
      let legacy = "FlightlineCode-" + name
      #expect(EVFontCatalog.face(named: legacy)?.italic == italic)
      #expect(EVFontCatalog.displayFamilyName(for: legacy) == "Flightline Code")
      let font = resolveFont(families: [legacy, "serif"], size: 17, cssWeight: CGFloat(weight), slant: 0, features: [])
      let url = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
      #expect(url?.lastPathComponent == "FlightlineCode-\(italic ? "Italic" : "Regular")-VF.ttf")
      #expect(CTFontGetSymbolicTraits(font).contains(.traitItalic) == italic)
      let coordinates = CTFontCopyVariation(font) as? [NSNumber: NSNumber] ?? [:]
      #expect(coordinates[NSNumber(value: EVFontVariations.identifier("wght"))]?.intValue ?? 400 == max(200, weight))
    }
    #expect(EVFontCatalog.faces(for: "FlightlineCode-does-not-exist").isEmpty)
    let recursive = EVFontVariations.info(for: "Recursive")
    #expect(Set(recursive.axes.map(\.tag)) == ["MONO", "CASL", "wght", "slnt", "CRSV"])
    #expect(recursive.instances.filter { $0.name != "Default" }.count == 64)
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
    #expect(count == 12 + recursive.instances.filter { $0.name != "Default" }.count)
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
