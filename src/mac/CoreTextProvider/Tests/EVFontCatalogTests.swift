import CViemCore
import CoreText
import Foundation
import Testing

@testable import ViemCoreTextProvider

@Suite("Font face and OpenType catalog")
struct EVFontCatalogTests {
  @Test func familyChangeRetainsStyleBeforeUsingRegularAnalogsOrFirstFace() throws {
    func face(_ style: String, weight: UInt16 = 400, italic: Bool = false) -> EVFontFace {
      EVFontFace(postScriptName: "New-\(style)", familyName: "New Family", styleName: style,
        weight: weight, italic: italic)
    }
    let current = EVFontFace(postScriptName: "Old-Light", familyName: "Old Family",
      styleName: "light", weight: 275, italic: false)
    let light = face("Light", weight: 300)
    let regular = face("Regular")
    let bold = face("Bold", weight: 700)
    func choose(from faces: [EVFontFace]) -> EVFontFace? {
      EVFontCatalog.preferredFace(currentFace: current, faces: faces)
    }
    #expect(choose(from: [regular, bold, light]) == light)
    #expect(choose(from: [bold, regular]) == regular)
    for style in ["Normal", "Roman", "Book"] {
      let upright = face(style)
      #expect(choose(from: [bold, upright]) == upright)
    }
    let oblique = face("Oblique", italic: true)
    #expect(choose(from: [oblique, bold]) == oblique)
    #expect(choose(from: []) == nil)
    let ambiguous = EVFontFace(postScriptName: "New Family", familyName: "New Family",
      styleName: "Regular", weight: 400, italic: false)
    #expect(choose(from: [ambiguous, light]) == light)
  }

  @Test func missingFamiliesExposeNoSubstituteFacesOrFeaturesAndUseOrderedFallback() {
    let missing = "Viem Missing Font \(UUID().uuidString)"
    #expect(EVFontCatalog.faces(for: missing).isEmpty)
    #expect(EVFontCatalog.face(named: missing) == nil)
    #expect(EVFontCatalog.faceForFamilyChange(to: missing, currentFace: nil) == nil)
    #expect(EVFontCatalog.features(for: missing).isEmpty)
    let font = resolveFont(families: [missing, "Georgia", "Helvetica"], size: 16,
      cssWeight: 400, slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [])
    #expect(CTFontCopyFamilyName(font) as String == "Georgia")
    let descriptor = CTFontCopyFontDescriptor(font)
    let cascade = CTFontDescriptorCopyAttribute(descriptor, kCTFontCascadeListAttribute) as? [CTFontDescriptor]
    #expect(cascade?.map { CTFontCopyFamilyName(CTFontCreateWithFontDescriptor($0, 16, nil)) as String } == ["Helvetica"])
  }

  @Test func faceNamesAreNotStylesheetFamilies() {
    let name = "HelveticaNeue-CondensedBold"
    #expect(EVFontCatalog.faces(for: name).isEmpty)
    #expect(EVFontCatalog.displayFamilyName(for: name) == name)
    #expect(EVFontCatalog.face(in: "Helvetica Neue", named: "") == nil)
    let font = resolveFont(families: [name, "Georgia"], size: 16,
      cssWeight: 400, slant: 0, features: [])
    #expect(CTFontCopyFamilyName(font) as String == "Georgia")
  }

  @Test func explicitFacesWithEqualWeightAndSlantKeepTheirNativeWidths() throws {
    let faces = EVFontCatalog.faces(for: "Helvetica Neue")
    let bold = try #require(faces.first { $0.styleName == "Bold" })
    let condensed = try #require(faces.first { $0.styleName == "Condensed Bold" })
    #expect(bold.weight == condensed.weight)
    #expect(bold.italic == condensed.italic)
    for face in [bold, condensed] {
      let portable = resolveFont(families: [face.familyName], size: 19,
        cssWeight: CGFloat(face.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [],
        faceName: face.styleName)
      #expect(CTFontCopyPostScriptName(portable) as String == face.postScriptName)
      #expect(abs(((CTFontCopyTraits(portable) as NSDictionary)[kCTFontWidthTrait] as? Double ?? 0) - face.width) < 0.0001)
    }
    let fallback = resolveFont(families: ["Viem Missing Font", "Helvetica Neue"], size: 19,
      cssWeight: CGFloat(condensed.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [],
      faceName: condensed.styleName)
    #expect(CTFontCopyPostScriptName(fallback) as String == bold.postScriptName)
  }

  @Test func privateSystemDescriptorsKeepCondensedFaceIdentityAndRelativeBold() throws {
    let faces = EVFontCatalog.faces(for: "SF Pro")
    let condensed = try #require(faces.first { $0.styleName == "Condensed Regular" })
    #expect(condensed.width < 0)
    let exact = resolveFont(families: [condensed.familyName], size: 19,
      cssWeight: CGFloat(condensed.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [], faceName: condensed.styleName)
    #expect(CTFontCopyPostScriptName(exact) as String == condensed.postScriptName)
    #expect(CTFontCopyFamilyName(exact) as String == condensed.familyName)
    let heavier = resolveFont(families: [condensed.familyName], size: 19,
      cssWeight: CGFloat(condensed.weight + 300), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT),
      features: [], relativeBold: true, faceName: condensed.styleName)
    #expect(EVFontCatalog.weight(of: heavier) > condensed.weight)
    #expect(abs(((CTFontCopyTraits(heavier) as NSDictionary)[kCTFontWidthTrait] as? Double ?? 0) - condensed.width) < 0.0001)
    EVFontCatalog.invalidate()
    let after = resolveFont(families: [condensed.familyName], size: 19,
      cssWeight: CGFloat(condensed.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [], faceName: condensed.styleName)
    #expect(CTFontCopyPostScriptName(after) as String == condensed.postScriptName)
  }

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
    #expect(EVFontCatalog.displayFamilyName(for: face.familyName) == "SF Pro")
    #expect(EVFontCatalog.displayFamilyName(for: ".AppleSystemUIFont") == "SF Pro")
    #expect(EVFontCatalog.face(named: face.postScriptName)?.postScriptName == face.postScriptName)
    #expect(EVFontCatalog.displayFamilyName(for: "Helvetica") == "Helvetica")
  }

  @Test func portableSystemFamiliesRoundTripThroughTheirPickerLabels() throws {
    #expect(EVFontCatalog.displayFamilyName(for: "system-ui") == "System Default")
    #expect(EVFontCatalog.displayFamilyName(for: "System-UI") == "System Default")
    #expect(EVFontCatalog.displayFamilyName(for: "ui-monospace") == "System Monospace")
    #expect(EVFontCatalog.portableFamily(forDisplayName: "System Default") == "system-ui")
    #expect(EVFontCatalog.portableFamily(forDisplayName: "system default") == "system-ui")
    #expect(EVFontCatalog.portableFamily(forDisplayName: "System Monospace") == "ui-monospace")
    #expect(EVFontCatalog.portableFamily(forDisplayName: "Helvetica") == nil)
    // Both tokens resolve to real, distinct system faces rather than an
    // unrecognized/empty family.
    #expect(!EVFontCatalog.faces(for: "system-ui").isEmpty)
    #expect(!EVFontCatalog.faces(for: "ui-monospace").isEmpty)
    let monospace = try #require(EVFontCatalog.faces(for: "ui-monospace").first)
    #expect(EVFontCatalog.faces(for: "monospace").contains(monospace))
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
      families: [light.familyName], size: 16,
      cssWeight: CGFloat(light.weight + 300), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT),
      features: [], relativeBold: true, faceName: light.styleName)
    #expect(EVFontCatalog.weight(of: font) == expected)
    let off = resolveFont(
      families: [light.familyName], size: 16,
      cssWeight: CGFloat(light.weight), slant: UInt32(VIEM_FONT_SLANT_UPRIGHT), features: [], faceName: light.styleName)
    #expect(CTFontCopyPostScriptName(off) as String == light.postScriptName)
  }
  @Test func catalogFeaturesAreFontOwnedAndDeterministic() {
    let features = EVFontCatalog.features(for: "Avenir Next")
    #expect(!features.isEmpty)
    #expect(features == EVFontCatalog.features(for: "Avenir Next"))
    #expect(Set(features.map(\.tag)).count == features.count)
    #expect(features.allSatisfy { $0.tag.utf8.count == 4 && !$0.label.isEmpty })
    #expect(!features.contains { ["init", "medi", "fina", "mark", "kern"].contains($0.tag) })
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
