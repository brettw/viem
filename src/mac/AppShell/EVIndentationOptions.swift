import Foundation

/// Persisted application defaults for portable hard-line indentation.
public struct EVIndentationOptions: Codable, Equatable, Sendable {
  public var autoindent = true
  public var tabstop: UInt32 = 2
  public var shiftwidth: UInt32 = 2
  public var softtabstop: Int32 = 2
  public var expandtab = true
  public var smarttab = true
  public var continueCommentsOnEnter = true
  public var continueCommentsOnOpenLine = true

  public init() {}

  public var isValid: Bool {
    (1...1024).contains(tabstop) && shiftwidth <= 1024 && (-1...1024).contains(softtabstop)
  }

  private enum CodingKeys: String, CodingKey {
    case autoindent, tabstop, shiftwidth, softtabstop, expandtab, smarttab
    case continueCommentsOnEnter, continueCommentsOnOpenLine
  }

  public init(from decoder: Decoder) throws {
    self.init()
    let values = try decoder.container(keyedBy: CodingKeys.self)
    autoindent = try values.decodeIfPresentStrict(Bool.self, forKey: .autoindent) ?? autoindent
    tabstop = try values.decodeIfPresentStrict(UInt32.self, forKey: .tabstop) ?? tabstop
    shiftwidth = try values.decodeIfPresentStrict(UInt32.self, forKey: .shiftwidth) ?? shiftwidth
    softtabstop = try values.decodeIfPresentStrict(Int32.self, forKey: .softtabstop) ?? softtabstop
    expandtab = try values.decodeIfPresentStrict(Bool.self, forKey: .expandtab) ?? expandtab
    smarttab = try values.decodeIfPresentStrict(Bool.self, forKey: .smarttab) ?? smarttab
    continueCommentsOnEnter = try values.decodeIfPresentStrict(Bool.self, forKey: .continueCommentsOnEnter) ?? continueCommentsOnEnter
    continueCommentsOnOpenLine = try values.decodeIfPresentStrict(Bool.self, forKey: .continueCommentsOnOpenLine) ?? continueCommentsOnOpenLine
  }
}

public enum EVWhitespaceWidth: String, Codable, Equatable, Sendable {
  case spaces
  case paragraphEn
}

/// Application-owned character declarations, using the core's sparse JSON schema.
public struct EVVisibleWhitespaceStyle: Codable, Equatable, Sendable {
  public enum Slant: String, Codable, Sendable { case upright = "Upright", italic = "Italic", oblique = "Oblique" }
  public enum Direction: String, Codable, Sendable { case natural = "Natural", leftToRight = "LeftToRight", rightToLeft = "RightToLeft" }
  public var fontFamilies: [String]?
  public var size: Float?
  public var weight: UInt16?
  public var bold: Bool?
  public var slant: Slant?
  public var foreground: EVThemeColor?
  public var background: EVThemeColor?
  public var underline: Bool?
  public var strikethrough: Bool?
  public var language: String?
  public var direction: Direction?
  public var openTypeFeatures: [String: UInt32]?
  public var letterSpacing: Float?
  public var baselineShift: Float?

  public init() {}

  public static var defaultStyle: Self {
    var result = Self()
    result.foreground = EVThemeColor(0, 0, Double(Float(139) / 255))
    return result
  }

  public var isValid: Bool {
    (fontFamilies.map { !$0.isEmpty && $0.allSatisfy { !$0.isEmpty } } ?? true)
      && (size.map { $0.isFinite && $0 > 0 } ?? true)
      && (weight.map { (1...1000).contains($0) } ?? true)
      && (foreground?.isValid ?? true) && (background?.isValid ?? true)
      && (language.map { !$0.isEmpty } ?? true)
      && (openTypeFeatures.map { $0.keys.allSatisfy { tag in
        tag.utf8.count == 4 && tag.utf8.allSatisfy { (0x20...0x7e).contains($0) }
      }} ?? true)
      && (letterSpacing?.isFinite ?? true) && (baselineShift?.isFinite ?? true)
  }

  public static let propertyNames = ["font_families", "size", "weight", "bold", "slant", "foreground", "background", "underline", "strikethrough", "language", "direction", "open_type_features", "letter_spacing", "baseline_shift"]

  private enum CodingKeys: String, CodingKey {
    case fontFamilies = "font_families", size, weight, bold, slant, foreground, background
    case underline, strikethrough, language, direction, openTypeFeatures = "open_type_features"
    case letterSpacing = "letter_spacing", baselineShift = "baseline_shift"
  }
}

public struct EVVisibleWhitespaceOptions: Codable, Equatable, Sendable {
  public static let defaultListchars = "tab:>-,trail:*,extends:>,precedes:<"
  public var enabled = true
  public var listchars = Self.defaultListchars
  public var style = EVVisibleWhitespaceStyle.defaultStyle
  public init() {}

  private enum CodingKeys: String, CodingKey { case enabled, listchars, style }
  public init(from decoder: Decoder) throws {
    self.init()
    let values = try decoder.container(keyedBy: CodingKeys.self)
    enabled = try values.decodeIfPresentStrict(Bool.self, forKey: .enabled) ?? enabled
    listchars = try values.decodeIfPresentStrict(String.self, forKey: .listchars) ?? listchars
    style = try values.decodeIfPresentStrict(EVVisibleWhitespaceStyle.self, forKey: .style) ?? style
  }
}

public struct EVWhitespacePresentationOptions: Codable, Equatable, Sendable {
  public var codeWhitespace = EVWhitespaceWidth.paragraphEn
  public var otherWhitespace = EVWhitespaceWidth.spaces
  public var codeWrappedLineIndent = 4
  public var visibleWhitespace = EVVisibleWhitespaceOptions()
  public init() {}

  public var isValid: Bool {
    (0...1024).contains(codeWrappedLineIndent)
      && visibleWhitespace.style.isValid && EVListcharsSettings.validationError(visibleWhitespace.listchars) == nil
  }

  private enum CodingKeys: String, CodingKey { case codeWhitespace, otherWhitespace, codeWrappedLineIndent, visibleWhitespace }
  public init(from decoder: Decoder) throws {
    self.init()
    let values = try decoder.container(keyedBy: CodingKeys.self)
    codeWhitespace = try values.decodeIfPresentStrict(EVWhitespaceWidth.self, forKey: .codeWhitespace) ?? codeWhitespace
    otherWhitespace = try values.decodeIfPresentStrict(EVWhitespaceWidth.self, forKey: .otherWhitespace) ?? otherWhitespace
    codeWrappedLineIndent = try values.decodeIfPresentStrict(Int.self, forKey: .codeWrappedLineIndent) ?? codeWrappedLineIndent
    visibleWhitespace = try values.decodeIfPresentStrict(EVVisibleWhitespaceOptions.self, forKey: .visibleWhitespace) ?? visibleWhitespace
  }
}

/// Editable names stay visible even when an entry is omitted from listchars.
public enum EVListcharsSettings {
  public static let names = ["eol", "tab", "space", "multispace", "lead", "leadmultispace", "leadtab", "trail", "extends", "precedes", "conceal", "nbsp"]

  public static func entries(_ value: String) -> [String: String] {
    var entries: [String: String] = [:]
    var parts = value.split(separator: ",", omittingEmptySubsequences: false)
    if parts.last?.isEmpty == true { parts.removeLast() }
    for part in parts {
      guard let separator = part.firstIndex(of: ":") else { continue }
      entries[String(part[..<separator])] = String(part[part.index(after: separator)...])
    }
    return entries
  }

  public static func string(from entries: [String: String]) -> String {
    names.compactMap { name in
      guard let value = entries[name], !value.isEmpty else { return nil }
      let encoded = value.replacingOccurrences(of: "\\", with: "\\x5c").replacingOccurrences(of: ",", with: "\\x2c")
      return "\(name):\(encoded)"
    }.joined(separator: ",")
  }

  public static func displayEntries(_ value: String) -> [String: String] {
    entries(value).mapValues { decodedCharacters($0) ?? $0 }
  }

  public static func validationError(_ value: String) -> String? {
    guard value.utf8.count <= 16_384 else { return "Visible whitespace characters must fit within 16 KiB." }
    if value.isEmpty { return nil }
    var parts = value.split(separator: ",", omittingEmptySubsequences: false)
    if parts.last?.isEmpty == true { parts.removeLast() }
    for part in parts {
      guard let separator = part.firstIndex(of: ":") else { return "Each visible whitespace entry requires a name and characters." }
      let name = String(part[..<separator])
      guard let characters = decodedCharacters(String(part[part.index(after: separator)...])) else { return "\(name) contains an invalid character escape." }
      guard names.contains(name) else { return "Unknown visible whitespace entry: \(name)." }
      let count = characters.unicodeScalars.count
      let validCount = ["tab", "leadtab"].contains(name) ? (2...3).contains(count)
        : ["multispace", "leadmultispace"].contains(name) ? count > 0 : count == 1
      guard validCount else { return "\(name) requires \(["tab", "leadtab"].contains(name) ? "two or three characters" : ["multispace", "leadmultispace"].contains(name) ? "at least one character" : "one character")." }
      guard characters.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) && !CharacterSet.nonBaseCharacters.contains($0) }) else {
        return "Visible whitespace entries require printable characters with their own width."
      }
    }
    let parsed = entries(value)
    if parsed["leadtab"] != nil && parsed["tab"] == nil { return "leadtab requires a tab entry." }
    return nil
  }

  static func decodedCharacters(_ text: String) -> String? {
    let scalars = Array(text.unicodeScalars)
    var output = String.UnicodeScalarView()
    var index = 0
    while index < scalars.count {
      if scalars[index] == "\\", index + 1 < scalars.count,
         let digits = ["x": 2, "u": 4, "U": 8][String(scalars[index + 1])] {
        guard index + 2 + digits <= scalars.count else { return nil }
        let hex = String(String.UnicodeScalarView(scalars[(index + 2)..<(index + 2 + digits)]))
        guard hex.utf8.allSatisfy({ (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }),
              let number = UInt32(hex, radix: 16), let scalar = Unicode.Scalar(number) else { return nil }
        output.append(scalar)
        index += 2 + digits
      } else {
        output.append(scalars[index])
        index += 1
      }
    }
    return String(output)
  }
}

private extension KeyedDecodingContainer {
  func decodeIfPresentStrict<T: Decodable>(_ type: T.Type, forKey key: Key) throws -> T? {
    contains(key) ? try decode(type, forKey: key) : nil
  }
}
