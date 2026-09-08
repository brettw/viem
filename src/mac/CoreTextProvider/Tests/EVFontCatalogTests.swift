import CViemCore
import CoreText
import Testing

@testable import ViemCoreTextProvider

@Suite("Font face and OpenType catalog")
struct EVFontCatalogTests {
  @Test func portableMonospaceResolvesFixedPitchAcrossTraitsAndGenerationChanges() {
    for bold in [false, true] {
      for slant in [VIEM_FONT_SLANT_UPRIGHT, VIEM_FONT_SLANT_ITALIC] {
        let font = resolveFont(families: ["monospace"], size: 14,
          cssWeight: bold ? 700 : 400, slant: UInt32(slant), features: [], relativeBold: bold)
        #expect(CTFontGetSymbolicTraits(font).contains(.traitMonoSpace))
        var characters: [UniChar] = [0x69, 0x57] // i and W have equal advances.
        var glyphs = [CGGlyph](repeating: 0, count: 2)
        #expect(CTFontGetGlyphsForCharacters(font, &characters, &glyphs, 2))
        var advances = [CGSize](repeating: .zero, count: 2)
        CTFontGetAdvancesForGlyphs(font, .horizontal, &glyphs, &advances, 2)
        #expect(abs(advances[0].width - advances[1].width) < 0.001)
      }
    }
    let before = EVFontCatalog.faces(for: "monospace")
    EVFontCatalog.invalidate()
    #expect(EVFontCatalog.faces(for: "monospace") == before)
    #expect(!before.isEmpty)
  }
  @Test func systemFaceDisplayNameKeepsPrivateFaceIdentity() throws {
    let face = try #require(
      EVFontCatalog.faces(for: "SF Pro").first { $0.postScriptName.hasPrefix(".SFNS") })
    #expect(EVFontCatalog.displayFamilyName(for: face.postScriptName) == "SF Pro")
    #expect(EVFontCatalog.displayFamilyName(for: ".AppleSystemUIFont") == "SF Pro")
    #expect(EVFontCatalog.face(named: face.postScriptName)?.postScriptName == face.postScriptName)
    #expect(EVFontCatalog.displayFamilyName(for: "Helvetica") == "Helvetica")
  }
  @Test func catalogUsesRealFacesAndWeightClasses() throws {
    let faces = EVFontCatalog.faces(for: "Avenir Next")
    #expect(faces.count > 3)
    #expect(Set(faces.map(\.postScriptName)).count == faces.count)
    #expect(
      faces.allSatisfy {
        !$0.familyName.isEmpty && !$0.styleName.isEmpty && (1...1000).contains($0.weight)
      })
    let light = try #require(faces.first { $0.weight < 400 && !$0.italic })
    let expected = EVFontCatalog.boldWeight(
      baseWeight: light.weight, faces: faces.filter { !$0.italic })
    let font = resolveFont(
      families: [light.postScriptName], size: 16,
      cssWeight: CGFloat(light.weight + 300), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT),
      features: [], relativeBold: true)
    #expect(EVFontCatalog.weight(of: font) == expected)
    let off = resolveFont(
      families: [light.postScriptName], size: 16,
      cssWeight: CGFloat(light.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
    #expect(CTFontCopyPostScriptName(off) as String == light.postScriptName)
  }
  @Test func catalogFeaturesAreFontOwnedAndDeterministic() {
    let features = EVFontCatalog.features(for: "Avenir Next")
    #expect(!features.isEmpty)
    #expect(features == EVFontCatalog.features(for: "Avenir Next"))
    #expect(Set(features.map(\.tag)).count == features.count)
    #expect(features.allSatisfy { $0.tag.utf8.count == 4 && !$0.label.isEmpty })
    #expect(!features.contains { ["init", "medi", "fina", "mark"].contains($0.tag) })
  }
  @Test func unavailableItalicRequestsSyntheticTreatment() {
    let font = resolveFont(
      families: ["Papyrus"], size: 16, cssWeight: 400,
      slant: UInt32(VIEM_FONT_SLANT_ITALIC), features: [])
    #expect(CTFontGetSymbolicTraits(font).contains(.traitItalic) || CTFontGetMatrix(font).c != 0)
  }
  @Test func generationChangeRetiresCatalogAndKeepsNewFaceRequestsValid() {
    let before = EVFontCatalog.faces(for: "Helvetica Neue")
    let provider = CoreTextMeasurementProvider(measurementEnvironmentID: 91)
    let generation = provider.metricsGeneration
    provider.invalidateMetrics()
    #expect(provider.metricsGeneration > generation)
    #expect(EVFontCatalog.faces(for: "Helvetica Neue") == before)
  }
}
