import AppKit
import CoreText
import Foundation

public struct EVFontFace: Equatable, Sendable {
  public let postScriptName: String
  public let familyName: String
  public let styleName: String
  public let weight: UInt16
  public let italic: Bool
  public let width: Double

  public init(postScriptName: String, familyName: String, styleName: String,
              weight: UInt16, italic: Bool, width: Double = 0) {
    self.postScriptName = postScriptName
    self.familyName = familyName
    self.styleName = styleName
    self.weight = weight
    self.italic = italic
    self.width = width
  }
}

public struct EVOpenTypeFeature: Equatable, Sendable {
  public let tag: String
  public let label: String
  public let defaultValue: UInt32
}

/// Native discovery only. Documents retain names and normalized properties,
/// never Core Text descriptors or objects.
public enum EVFontCatalog {
  /// Register packaged files before constructing font pickers or shapers. The
  /// registration lasts for this process only; missing resources leave ordinary
  /// system-font fallback available. No native work runs under the catalog lock.
  @discardableResult
  public static func registerBundledFonts(in directory: URL) -> [String] {
    guard FileManager.default.fileExists(atPath: directory.path) else { return [] }
    var failures: [String] = []
    guard let files = FileManager.default.enumerator(
      at: directory, includingPropertiesForKeys: [.isRegularFileKey], options: [.skipsHiddenFiles],
      errorHandler: { url, error in
        failures.append("\(url.lastPathComponent): \(error.localizedDescription)")
        return true
      }) else { return ["Could not read \(directory.path)"] }
    let urls = files.compactMap { $0 as? URL }.filter {
      ["ttf", "otf", "ttc", "otc"].contains($0.pathExtension.lowercased())
        && (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
    }.sorted { $0.path < $1.path }
    for url in urls {
      var error: Unmanaged<CFError>?
      if !CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error),
        let failure = error?.takeRetainedValue(),
        CFErrorGetCode(failure) != CTFontManagerError.alreadyRegistered.rawValue
      {
        failures.append("\(url.lastPathComponent): \(failure)")
      }
    }
    if !urls.isEmpty { invalidate() }
    return failures
  }

  private final class Cache: @unchecked Sendable {
    let lock = NSLock()
    var generation: UInt64 = 1
    var faces: [String: [EVFontFace]] = [:]
    var descriptors: [String: CTFontDescriptor] = [:]
  }
  private static let cache = Cache()
  public static func invalidate() {
    cache.lock.lock()
    cache.generation &+= 1
    cache.faces.removeAll()
    cache.descriptors.removeAll()
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
    if generation == cache.generation {
      cache.faces[familyOrPostScriptName] = result.faces
      cache.descriptors.merge(result.descriptors) { _, new in new }
    }
    cache.lock.unlock()
    return result.faces
  }

  private struct Discovery {
    var faces: [EVFontFace] = []
    var descriptors: [String: CTFontDescriptor] = [:]
  }

  private static func matchingDescriptors(_ key: CFString, _ value: String) -> [CTFontDescriptor] {
    let request = CTFontDescriptorCreateWithAttributes(
      [key: value] as CFDictionary)
    return CTFontDescriptorCreateMatchingFontDescriptors(request, NSSet(object: key) as CFSet)
      as? [CTFontDescriptor] ?? []
  }

  private static func discoverFaces(for requested: String) -> Discovery {
    let system = systemFont(named: requested, size: 14)
    var family = system.map { CTFontCopyFamilyName($0) as String } ?? requested
    var descriptors = matchingDescriptors(kCTFontFamilyNameAttribute, family)
    if descriptors.isEmpty {
      // Matching by name can return substitutes too. Accept only an exact
      // installed face before asking for its family; unknown names stay empty.
      guard let descriptor = matchingDescriptors(kCTFontNameAttribute, requested).first(where: {
        sameName(CTFontCopyPostScriptName(CTFontCreateWithFontDescriptor($0, 14, nil)) as String, requested)
      }) else { return Discovery() }
      let font = CTFontCreateWithFontDescriptor(descriptor, 14, nil)
      family = CTFontCopyFamilyName(font) as String
      descriptors = matchingDescriptors(kCTFontFamilyNameAttribute, family)
      if descriptors.isEmpty { descriptors = [descriptor] }
    }
    var result = Discovery()
    for descriptor in descriptors {
      let candidate = CTFontCreateWithFontDescriptor(descriptor, 14, nil)
      let name = CTFontCopyPostScriptName(candidate) as String
      guard sameName(CTFontCopyFamilyName(candidate) as String, family),
        result.descriptors[name] == nil else { continue }
      result.descriptors[name] = descriptor
      result.faces.append(EVFontFace(
        postScriptName: name,
        familyName: CTFontCopyFamilyName(candidate) as String,
        styleName: CTFontCopyName(candidate, kCTFontStyleNameKey) as String? ?? name,
        weight: weight(of: candidate),
        italic: CTFontGetSymbolicTraits(candidate).contains(.traitItalic),
        width: (CTFontCopyTraits(candidate) as NSDictionary)[kCTFontWidthTrait] as? Double ?? 0))
    }
    if requested.hasPrefix(".SFNS-"), !result.faces.contains(where: { sameName($0.postScriptName, requested) }) {
      return Discovery()
    }
    result.faces.sort {
      if $0.weight != $1.weight { return $0.weight < $1.weight }
      if $0.italic != $1.italic { return !$0.italic }
      let styleOrder = $0.styleName.localizedStandardCompare($1.styleName)
      if styleOrder != .orderedSame { return styleOrder == .orderedAscending }
      return $0.postScriptName < $1.postScriptName
    }
    return result
  }

  public static func face(named name: String) -> EVFontFace? {
    faces(for: name).first { sameName($0.postScriptName, name) }
  }

  public static func faceForFamilyChange(to familyOrFace: String, currentFace: EVFontFace?) -> EVFontFace? {
    faceForFamilyChange(to: familyOrFace, currentFace: currentFace, faces: faces(for: familyOrFace))
  }

  static func faceForFamilyChange(to requested: String, currentFace: EVFontFace?, faces: [EVFontFace]) -> EVFontFace? {
    // Some Regular PostScript names equal their family (e.g. Helvetica). The
    // family picker treats those as a family; the face picker retains exact IDs.
    if let explicit = faces.first(where: { sameName($0.postScriptName, requested) && !sameName($0.familyName, requested) }) {
      return explicit
    }
    if let currentFace, let corresponding = faces.first(where: { sameName($0.styleName, currentFace.styleName) }) {
      return corresponding
    }
    for style in ["Regular", "Normal", "Roman", "Book"] {
      if let regular = faces.first(where: { !$0.italic && sameName($0.styleName, style) }) { return regular }
    }
    return faces.first
  }

  /// Retain Core Text's matched descriptors: private and variable named faces
  /// cannot always be reconstructed from their PostScript spelling.
  static func font(for face: EVFontFace, size: CGFloat) -> CTFont? {
    _ = faces(for: face.familyName)
    cache.lock.lock()
    let descriptor = cache.descriptors[face.postScriptName]
    cache.lock.unlock()
    let matched = descriptor ?? discoverFaces(for: face.familyName).descriptors[face.postScriptName]
    return matched.map { CTFontCreateWithFontDescriptor($0, size, nil) }
  }

  private static func sameName(_ first: String, _ second: String) -> Bool {
    first.caseInsensitiveCompare(second) == .orderedSame
  }

  /// Portable request tokens for the family picker's built-in "use the
  /// system font" entries. FontCatalog.Resolve on Windows recognizes the
  /// identical strings, so a style saved with one renders consistently on
  /// both platforms; only the picker's label differs from a literal font name.
  public static let systemDefaultFamily = "system-ui"
  public static let systemMonospaceFamily = "ui-monospace"
  private static let systemFamilyLabels: [(family: String, label: String)] = [
    (systemDefaultFamily, "System Default"),
    (systemMonospaceFamily, "System Monospace"),
  ]

  /// Friendly presentation only; selected PostScript identities remain exact
  /// in document requests and in the face catalog.
  public static func displayFamilyName(for familyOrFace: String) -> String {
    if let label = systemFamilyLabels.first(where: { sameName($0.family, familyOrFace) })?.label {
      return label
    }
    let family = face(named: familyOrFace)?.familyName ?? familyOrFace
    if familyOrFace.hasPrefix(".SFNS") || family.hasPrefix(".AppleSystemUIFont")
      || ["system-ui", "sf pro", "-apple-system"].contains(family.lowercased())
    {
      return "SF Pro"
    }
    return family
  }

  /// Reverses `displayFamilyName` for the picker's own two entries; any other
  /// text (an installed font name, typed or picked) passes through unchanged.
  public static func portableFamily(forDisplayName label: String) -> String? {
    systemFamilyLabels.first { sameName($0.label, label) }?.family
  }

  public static func boldWeight(baseWeight: UInt16, faces: [EVFontFace]) -> UInt16 {
    let target = min(Int(baseWeight) + 300, 1000)
    return faces.map(\.weight).filter { Int($0) >= target }.min() ?? UInt16(target)
  }

  public static func features(for familyOrFace: String) -> [EVOpenTypeFeature] {
    guard let font = availableFont(named: familyOrFace, size: 14) else { return [] }
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
      "fin2", "fin3", "med2", "rvrn", "stch", "kern",
    ]
    return tags.subtracting(required).sorted().map {
      EVOpenTypeFeature(
        tag: $0, label: labels[$0] ?? featureLabel($0),
        defaultValue: ["liga", "clig", "calt"].contains($0) ? 1 : 0)
    }
  }

  static func baseFont(named name: String, size: CGFloat) -> CTFont {
    availableFont(named: name, size: size)
      ?? CTFontCreateUIFontForLanguage(.system, size, nil)
      ?? CTFontCreateWithName("Helvetica" as CFString, size, nil)
  }

  static func availableFont(named name: String, size: CGFloat) -> CTFont? {
    let available = faces(for: name)
    let face = available.first { sameName($0.postScriptName, name) }
      ?? faceForFamilyChange(to: name, currentFace: nil, faces: available)
    return face.flatMap { font(for: $0, size: size) }
  }

  private static func systemFont(named name: String, size: CGFloat) -> CTFont? {
    if ["monospace", "ui-monospace", "system monospace", "systemmonospace"].contains(name.lowercased()) {
      return NSFont.monospacedSystemFont(ofSize: size, weight: .regular) as CTFont
    }
    if ["system-ui", "sf pro", "-apple-system"].contains(name.lowercased())
      || (name.hasPrefix(".SFNS") && !name.hasPrefix(".SFNSMono"))
    {
      return CTFontCreateUIFontForLanguage(.system, size, nil)
        ?? CTFontCreateWithName("Helvetica" as CFString, size, nil)
    }
    return nil
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
    "liga": "Standard Ligatures", "dlig": "Discretionary Ligatures",
    "hlig": "Historical Ligatures", "clig": "Contextual Ligatures", "calt": "Contextual Alternates",
    "smcp": "Small Capitals", "c2sc": "Capitals to Small Capitals", "pcap": "Petite Capitals",
    "c2pc": "Capitals to Petite Capitals", "case": "Case Sensitive Forms",
    "cpsp": "Capital Spacing",
    "onum": "Oldstyle Figures", "lnum": "Lining Figures", "pnum": "Proportional Figures",
    "tnum": "Tabular Figures", "frac": "Fractions", "afrc": "Alternative Fractions",
    "zero": "Slashed Zero", "ordn": "Ordinals",
    "sinf": "Scientific Inferiors", "swsh": "Swash", "cswh": "Contextual Swash",
    "salt": "Stylistic Alternates", "titl": "Titling Alternates", "hist": "Historical Forms",
  ]
}
