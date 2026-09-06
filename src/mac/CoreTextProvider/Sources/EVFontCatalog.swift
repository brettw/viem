import AppKit
import CoreText
import Foundation

public struct EVFontFace: Equatable, Sendable {
  public let postScriptName: String
  public let familyName: String
  public let styleName: String
  public let weight: UInt16
  public let italic: Bool
}

public struct EVOpenTypeFeature: Equatable, Sendable {
  public let tag: String
  public let label: String
  public let defaultValue: UInt32
}

/// Native discovery only. Documents retain names and normalized properties,
/// never Core Text descriptors or objects.
public enum EVFontCatalog {
  private final class Cache: @unchecked Sendable {
    let lock = NSLock()
    var generation: UInt64 = 1
    var faces: [String: [EVFontFace]] = [:]
  }
  private static let cache = Cache()
  public static func invalidate() {
    cache.lock.lock()
    cache.generation &+= 1
    cache.faces.removeAll()
    cache.lock.unlock()
  }
  public static func faces(for familyOrPostScriptName: String) -> [EVFontFace] {
    cache.lock.lock()
    let generation = cache.generation
    let cached = cache.faces[familyOrPostScriptName]
    cache.lock.unlock()
    if let cached { return cached }
    let result = discoverFaces(for: familyOrPostScriptName)
    cache.lock.lock()
    if generation == cache.generation { cache.faces[familyOrPostScriptName] = result }
    cache.lock.unlock()
    return result
  }
  private static func discoverFaces(for familyOrPostScriptName: String) -> [EVFontFace] {
    let font = baseFont(named: familyOrPostScriptName, size: 14)
    let family = CTFontCopyFamilyName(font) as String
    let request = CTFontDescriptorCreateWithAttributes(
      [
        kCTFontFamilyNameAttribute: family
      ] as CFDictionary)
    let descriptors =
      CTFontDescriptorCreateMatchingFontDescriptors(
        request, NSSet(object: kCTFontFamilyNameAttribute) as CFSet
      ) as? [CTFontDescriptor] ?? [CTFontCopyFontDescriptor(font)]
    var seen = Set<String>()
    return descriptors.compactMap { descriptor -> EVFontFace? in
      let candidate = CTFontCreateWithFontDescriptor(descriptor, 14, nil)
      let name = CTFontCopyPostScriptName(candidate) as String
      guard seen.insert(name).inserted else { return nil }
      return EVFontFace(
        postScriptName: name,
        familyName: CTFontCopyFamilyName(candidate) as String,
        styleName: CTFontCopyName(candidate, kCTFontStyleNameKey) as String? ?? name,
        weight: weight(of: candidate),
        italic: CTFontGetSymbolicTraits(candidate).contains(.traitItalic))
    }.sorted {
      if $0.weight != $1.weight { return $0.weight < $1.weight }
      if $0.italic != $1.italic { return !$0.italic }
      return $0.styleName.localizedStandardCompare($1.styleName) == .orderedAscending
    }
  }

  public static func face(named name: String) -> EVFontFace? {
    faces(for: name).first { $0.postScriptName == name }
  }

  /// Friendly presentation only; selected PostScript identities remain exact
  /// in document requests and in the face catalog.
  public static func displayFamilyName(for familyOrFace: String) -> String {
    let family = face(named: familyOrFace)?.familyName ?? familyOrFace
    if familyOrFace.hasPrefix(".SFNS") || family.hasPrefix(".AppleSystemUIFont")
      || ["system-ui", "sf pro", "-apple-system"].contains(family.lowercased())
    {
      return "SF Pro"
    }
    return family
  }

  public static func boldWeight(baseWeight: UInt16, faces: [EVFontFace]) -> UInt16 {
    let target = min(Int(baseWeight) + 300, 1000)
    return faces.map(\.weight).filter { Int($0) >= target }.min() ?? UInt16(target)
  }

  public static func features(for familyOrFace: String) -> [EVOpenTypeFeature] {
    let font = baseFont(named: familyOrFace, size: 14)
    var tags = Set<String>()
    for table in [CTFontTableTag(0x4753_5542), CTFontTableTag(0x4750_4F53)] {
      guard let data = CTFontCopyTable(font, table, []) as Data? else { continue }
      let bytes = [UInt8](data)
      func u16(_ at: Int) -> Int? {
        guard at >= 0, at + 1 < bytes.count else { return nil }
        return Int(bytes[at]) << 8 | Int(bytes[at + 1])
      }
      guard let start = u16(6), let count = u16(start),
        count <= (bytes.count - min(bytes.count, start + 2)) / 6
      else { continue }
      for index in 0..<count {
        let at = start + 2 + index * 6
        if let tag = String(bytes: bytes[at..<at + 4], encoding: .ascii) { tags.insert(tag) }
      }
    }
    // Required shaping features remain under the shaper's control. Expose
    // selectable typographic features rather than script joining machinery.
    let required: Set<String> = [
      "ccmp", "locl", "rlig", "rclt", "curs", "mark", "mkmk", "isol", "init", "medi", "fina",
      "fin2", "fin3", "med2", "rvrn", "stch",
    ]
    return tags.subtracting(required).sorted().map {
      EVOpenTypeFeature(
        tag: $0, label: labels[$0] ?? featureLabel($0),
        defaultValue: ["kern", "liga", "clig", "calt"].contains($0) ? 1 : 0)
    }
  }

  static func baseFont(named name: String, size: CGFloat) -> CTFont {
    if ["monospace", "ui-monospace", "system monospace", "systemmonospace"].contains(name.lowercased()) {
      return NSFont.monospacedSystemFont(ofSize: size, weight: .regular) as CTFont
    }
    if ["system-ui", "sf pro", "-apple-system"].contains(name.lowercased())
      || (name.hasPrefix(".SFNS") && !name.hasPrefix(".SFNSMono"))
    {
      return CTFontCreateUIFontForLanguage(.system, size, nil)
        ?? CTFontCreateWithName("Helvetica" as CFString, size, nil)
    }
    return CTFontCreateWithName(name as CFString, size, nil)
  }

  public static func weight(of font: CTFont) -> UInt16 {
    // OS/2 records the actual CSS/OpenType weight class, unlike Core Text's
    // nonlinear normalized trait value. Variable named instances override it.
    if let variations = CTFontCopyVariation(font) as? [NSNumber: NSNumber],
      let value = variations[NSNumber(value: 0x7767_6874)]?.doubleValue,
      value >= 1, value <= 1000
    {
      return UInt16(value.rounded())
    }
    if let table = CTFontCopyTable(font, CTFontTableTag(0x4F53_2F32), []) as Data?, table.count >= 6
    {
      let value = UInt16(table[4]) << 8 | UInt16(table[5])
      if (1...1000).contains(value) { return value }
    }
    let trait = (CTFontCopyTraits(font) as NSDictionary)[kCTFontWeightTrait] as? Double ?? 0
    let anchors: [(Double, UInt16)] = [
      (-0.8, 100), (-0.6, 200), (-0.4, 300), (0, 400), (0.23, 500), (0.3, 600), (0.4, 700),
      (0.56, 800), (0.62, 900),
    ]
    return anchors.min { abs($0.0 - trait) < abs($1.0 - trait) }!.1
  }

  private static func featureLabel(_ tag: String) -> String {
    if tag.hasPrefix("ss"), let number = Int(tag.dropFirst(2)) { return "Stylistic Set \(number)" }
    if tag.hasPrefix("cv"), let number = Int(tag.dropFirst(2)) {
      return "Character Variant \(number)"
    }
    return tag
  }
  private static let labels = [
    "kern": "Kerning", "liga": "Standard Ligatures", "dlig": "Discretionary Ligatures",
    "hlig": "Historical Ligatures", "clig": "Contextual Ligatures", "calt": "Contextual Alternates",
    "smcp": "Small Capitals", "c2sc": "Capitals to Small Capitals", "pcap": "Petite Capitals",
    "c2pc": "Capitals to Petite Capitals", "case": "Case Sensitive Forms",
    "cpsp": "Capital Spacing",
    "onum": "Oldstyle Figures", "lnum": "Lining Figures", "pnum": "Proportional Figures",
    "tnum": "Tabular Figures", "frac": "Fractions", "afrc": "Alternative Fractions",
    "zero": "Slashed Zero", "ordn": "Ordinals", "sups": "Superscript", "subs": "Subscript",
    "sinf": "Scientific Inferiors", "swsh": "Swash", "cswh": "Contextual Swash",
    "salt": "Stylistic Alternates", "titl": "Titling Alternates", "hist": "Historical Forms",
  ]
}
