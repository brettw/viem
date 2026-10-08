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
    let availableNames = Set(CTFontManagerCopyAvailablePostScriptNames() as? [String] ?? [])
    let bundled = Set(EVFontCatalog.fontsRequiringRegistration(files, availableNames: availableNames))
    var installedURLs: [String: URL] = [:]
    for file in files where !bundled.contains(file) {
      let descriptors = try #require(CTFontManagerCreateFontDescriptorsFromURL(file as CFURL) as? [CTFontDescriptor])
      for descriptor in descriptors {
        let name = try #require(CTFontDescriptorCopyAttribute(descriptor, kCTFontNameAttribute) as? String)
        let request = CTFontDescriptorCreateWithAttributes([kCTFontNameAttribute: name] as CFDictionary)
        let installed = try #require(CTFontDescriptorCreateMatchingFontDescriptor(request, NSSet(object: kCTFontNameAttribute) as CFSet))
        installedURLs[name] = try #require(CTFontDescriptorCopyAttribute(installed, kCTFontURLAttribute) as? URL)
      }
    }
    defer {
      for file in bundled { CTFontManagerUnregisterFontsForURL(file as CFURL, .process, nil) }
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
    #expect(EVFontCatalog.faces(for: "Flightline Code").count >= 12)
    #expect(Set(files.filter { $0.deletingLastPathComponent().lastPathComponent == "flightline" }
      .map(\.lastPathComponent)) == ["FlightlineCode-Regular-VF.ttf", "FlightlineCode-Italic-VF.ttf"])
    #expect(EVFontCatalog.faces(for: "FlightlineCode-does-not-exist").isEmpty)
    var count = 0
    for file in files {
      let descriptors = try #require(CTFontManagerCreateFontDescriptorsFromURL(file as CFURL) as? [CTFontDescriptor])
      for descriptor in descriptors {
        let originalFont = CTFontCreateWithFontDescriptor(descriptor, 17, nil)
        // Inspect packaged metadata directly: matching installed faces may be
        // another version (including static fonts) under the same names.
        let variations = EVFontVariations.info(font: originalFont)
        if file.deletingLastPathComponent().lastPathComponent == "flightline" {
          #expect(variations.axes.count == 1)
          #expect(variations.axes.first?.tag == "wght")
          #expect(variations.axes.first?.minimum == 200)
          #expect(variations.axes.first?.defaultValue == 400)
          #expect(variations.axes.first?.maximum == 700)
          #expect(variations.instances.filter { $0.name != "Default" }.count == 6)
        } else {
          #expect(Set(variations.axes.map(\.tag)) == ["MONO", "CASL", "wght", "slnt", "CRSV"])
          #expect(variations.instances.filter { $0.name != "Default" }.count == 64)
        }
        let name = CTFontCopyPostScriptName(originalFont) as String
        let face = try #require(EVFontCatalog.face(named: name))
        let font = resolveFont(families: [face.familyName], size: 17, cssWeight: CGFloat(face.weight),
          slant: face.italic ? UInt32(VIEM_FONT_SLANT_ITALIC) : UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [], faceName: face.styleName)
        #expect(CTFontCopyPostScriptName(font) as String == name)
        #expect(EVFontCatalog.weight(of: font) == face.weight)
        let url = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
        #expect(url?.standardizedFileURL == (installedURLs[name] ?? file).standardizedFileURL)
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
    #expect(count == 12 + 64)
  }

  @Test func installedNamesCoverEveryFaceBeforeSkippingAFile() throws {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    let directory = root.appendingPathComponent("assets/fonts/flightline")
    let upright = directory.appendingPathComponent("FlightlineCode-Regular-VF.ttf")
    let italic = directory.appendingPathComponent("FlightlineCode-Italic-VF.ttf")
    let files = [upright, italic]
    func names(in url: URL) throws -> Set<String> {
      let descriptors = try #require(CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor])
      return Set(try descriptors.map { try #require(CTFontDescriptorCopyAttribute($0, kCTFontNameAttribute) as? String) })
    }
    let uprightNames = try names(in: upright)
    let italicNames = try names(in: italic)
    let allNames = uprightNames.union(italicNames)
    #expect(EVFontCatalog.fontsRequiringRegistration(files, availableNames: allNames).isEmpty)
    #expect(EVFontCatalog.fontsRequiringRegistration(files, availableNames: []).count == 2)
    #expect(EVFontCatalog.fontsRequiringRegistration(files, availableNames: ["Flightline Code"]) == files)
    #expect(EVFontCatalog.fontsRequiringRegistration(files, availableNames: uprightNames) == [italic])
    let missingWeight = try #require(uprightNames.first)
    #expect(EVFontCatalog.fontsRequiringRegistration(files, availableNames: allNames.subtracting([missingWeight])) == [upright])
  }

  @Test func missingOrInvalidFontsLeaveSystemFallbackAvailable() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("Viem invalid fonts \(UUID().uuidString)")
    #expect(EVFontCatalog.registerBundledFonts(in: directory).isEmpty)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let broken = directory.appendingPathComponent("broken.ttf")
    try Data("invalid font".utf8).write(to: broken)
    #expect(EVFontCatalog.fontsRequiringRegistration([broken], availableNames: ["broken"]) == [broken])
    #expect(EVFontCatalog.registerBundledFonts(in: directory).count == 1)
    #expect(!EVFontCatalog.faces(for: "system-ui").isEmpty)
  }
}
