import AppKit
import CViemCore

/// Process/profile-local Vim settings. Script configuration never rewrites the
/// settings file or a theme merely because a document was opened.
@MainActor
public final class EVScriptApplicationOptions {
  public var startupApplicationOptionsApplied = false
  public var timeout = true
  public var timeoutLength = 1000
  public var errorbells = true
  public var visualbell = false
  public var belloff = ""
  public var scrolloff: Int?
  public var showStatusBar: Bool?
  public var breakindent = true
  public var breakindentShift = 4
  public var continuationIndent: Int?

  public init() {}
  public var audibleBell: Bool { errorbells && !visualbell && belloff.isEmpty }
  public func beep() { if audibleBell { NSSound.beep() } }

  public func apply(_ name: String, value: String) throws {
    func number(_ range: ClosedRange<Int>) throws -> Int {
      guard let result = Int(value), range.contains(result) else {
        throw EVConfigurationStore.invalid("Invalid value for '\(name)': \(value)")
      }
      return result
    }
    switch name {
    case "timeout": timeout = value == "1"
    case "timeoutlen": timeoutLength = try number(0...Int(Int32.max))
    case "errorbells": errorbells = value == "1"
    case "visualbell": visualbell = value == "1"
    case "belloff": belloff = value
    case "scrolloff": scrolloff = try number(0...1_000_000)
    case "laststatus": showStatusBar = try number(0...3) != 0
    case "breakindent":
      breakindent = value == "1"; continuationIndent = breakindent ? breakindentShift : 0
    case "breakindentopt":
      // Viem's Code continuation margin is the portable counterpart of shift.
      // Other Vim wrapping rules have no equivalent and must not be accepted.
      let shift: Int
      if value.isEmpty { shift = 0 }
      else if value.hasPrefix("shift:"), let parsed = Int(value.dropFirst(6)), (0...1024).contains(parsed) { shift = parsed }
      else { throw EVConfigurationStore.invalid("Viem supports breakindentopt=shift:0…1024") }
      breakindentShift = shift; continuationIndent = breakindent ? shift : 0
    default: throw EVConfigurationStore.invalid("Unsupported application option '\(name)'")
    }
  }
}

extension EVConfigurationStore {
  /// Font changes are disposable Code presentation, shared by Code documents.
  /// Validate the entire candidate before publishing it to observers.
  func applyScriptCodeOption(_ name: String, value: String) throws {
    var candidate = activeTheme
    var styles = candidate["styles"] as? [String: Any] ?? [:]
    var code = styles["code"] as? [String: Any] ?? [:]
    var blocks = code["block_styles"] as? [[String: Any]] ?? []
    guard let index = blocks.firstIndex(where: { $0["id"] as? String == "Paragraph" }) else {
      throw Self.invalid("The Code base paragraph is unavailable")
    }
    var paragraph = blocks[index]
    var character = paragraph["character"] as? [String: Any] ?? [:]
    if name == "guifont" {
      let fonts = try EVScriptFont.parse(value)
      character["font_families"] = fonts.families
      character["font_face"] = ""
      character["font_axes"] = [String: Double]()
      if let size = fonts.size { character["size"] = size }
      character["weight"] = fonts.bold ? 700 : 400
      character["bold"] = fonts.bold
      character["slant"] = fonts.italic ? "Italic" : "Upright"
      paragraph["character"] = character
    } else {
      guard let spacing = Double(value), spacing.isFinite, (0...1000).contains(spacing) else { throw Self.invalid("Invalid linespace") }
      let size = (character["size"] as? NSNumber)?.doubleValue ?? 14
      var block = paragraph["block"] as? [String: Any] ?? [:]
      block["line_spacing"] = spacing == 0 ? "Normal" : ["AtLeast": size * 1.2 + spacing] as Any
      paragraph["block"] = block
    }
    blocks[index] = paragraph; code["block_styles"] = blocks
    styles["code"] = code; candidate["styles"] = styles
    _ = try EVThemeFile.encode(candidate)
    activeTheme = candidate
    NotificationCenter.default.post(name: .viemThemeDidChange, object: self,
      userInfo: ["selectionChanged": false, "styleNames": ["code"]])
  }
}

struct EVScriptFont {
  var families: [String] = []
  var size: Double?
  var bold = false
  var italic = false
  @MainActor static func parse(_ value: String) throws -> Self {
    if value.isEmpty { return Self(families: ["monospace"]) }
    var result = Self()
    for spelling in value.split(separator: ",", omittingEmptySubsequences: false) {
      let fields = spelling.split(separator: ":", omittingEmptySubsequences: false)
      guard let first = fields.first, !first.isEmpty else { throw EVConfigurationStore.invalid("Invalid guifont") }
      result.families.append(String(first).replacingOccurrences(of: "_", with: " "))
      for field in fields.dropFirst() {
        if field == "b" { result.bold = true }
        else if field == "i" { result.italic = true }
        else if field.hasPrefix("h"), let size = Double(field.dropFirst()), size.isFinite, (1...1000).contains(size) { result.size = size }
        else { throw EVConfigurationStore.invalid("Viem supports guifont families with :h<size>, :b and :i") }
      }
    }
    return result
  }
}
