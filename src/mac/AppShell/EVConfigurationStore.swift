import Foundation
import CoreFoundation

extension Notification.Name {
  public static let viemConfigurationDidChange = Notification.Name("com.viem.configuration.did-change")
}

public struct EVCodeFilenameAssociation: Codable, Equatable, Sendable {
  public var pattern: String
  public var language: String
  public init(pattern: String, language: String) {
    self.pattern = pattern
    self.language = language
  }
}

/// One application-wide settings authority. Unknown keys are retained so older
/// versions do not erase newer preferences. File I/O is the native mechanism;
/// document style validation and cascades live in the portable core.
@MainActor
public final class EVConfigurationStore {
  public static let shared = EVConfigurationStore()
  public let directory: URL
  /// All buffers share the first read of startup.viem until this profile is
  /// reopened, normally at the next application launch.
  public private(set) lazy var startupFile = EVStartupFile.load(directory: directory)
  public internal(set) var lastError: String?
  var root: [String: Any] = ["version": 1]
  var writable = true
  let manager: FileManager
  let bundleResourceURL: URL?

  var activeTheme: [String: Any] = [:]
  var activeThemeName: String?
  var activeThemeFile: URL?
  var activeThemeDiskData: Data?
  private var reloadingFromDisk = false

  public init(directory: URL? = nil, legacyDefaults: UserDefaults? = nil,
              manager: FileManager = .default,
              environment: [String: String] = ProcessInfo.processInfo.environment,
              homeDirectory: URL? = nil,
              bundleResourceURL: URL? = Bundle.main.resourceURL) {
    self.manager = manager
    self.bundleResourceURL = bundleResourceURL
    let profile = EVProfileDirectory.resolve(directory: directory, environment: environment,
      homeDirectory: homeDirectory ?? manager.homeDirectoryForCurrentUser)
    // Injected directories are isolated (tests, previews, portable profiles):
    // never consume the real application's legacy preferences there.
    let legacy = legacyDefaults ?? (profile.usesDefaultDirectory && homeDirectory == nil ? UserDefaults.standard : nil)
    self.directory = profile.url
    let file = self.directory.appendingPathComponent("config.json")
    let newProfile = !manager.fileExists(atPath: file.path)
    activeTheme = (try? EVThemeFile.builtin()) ?? [:]
    do {
      if manager.fileExists(atPath: file.path) {
        root = try Self.readObject(Data(contentsOf: file))
        try Self.validate(root)
      } else {
        if let value = legacy?.object(forKey: "EVEditing.SmartQuotes.v1") as? Bool {
          root["editing"] = ["smartQuotes": value]
        }
        if let value = legacy?.object(forKey: "EVShowStatusBar") as? Bool {
          root["appearance"] = ["showStatusBar": value]
        }
        if root.count > 1 {
          try write(root, to: file)
          ["EVEditing.SmartQuotes.v1", "EVShowStatusBar"].forEach {
            legacy?.removeObject(forKey: $0)
          }
        }
      }
    } catch {
      lastError = error.localizedDescription
      writable = false
      root = ["version": 1]
    }
    if writable {
      do { try initializeThemes(newProfile: newProfile) }
      catch {
        lastError = error.localizedDescription
        activeTheme = (try? EVThemeFile.builtin()) ?? [:]
        activeThemeName = nil
        activeThemeFile = nil
        activeThemeDiskData = nil
      }
    }
  }

  public var theme: EVTheme {
    guard let value = activeTheme["theme"], let data = try? JSONSerialization.data(withJSONObject: value),
          let result = try? JSONDecoder().decode(EVTheme.self, from: data), result.isValid else { return .midnight }
    return result
  }
  public var viewMargins: EVViewMargins {
    guard let value = (root["view"] as? [String: Any])?["margins"],
          let data = try? JSONSerialization.data(withJSONObject: value),
          let result = try? JSONDecoder().decode(EVViewMargins.self, from: data), result.isValid
    else { return EVViewMargins() }
    return result
  }
  public func setViewMargins(_ margins: EVViewMargins) throws {
    guard margins.isValid else { throw invalid("View margins must be between 0 and 1000 pixels") }
    let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(margins))
    try update(section: "view", values: ["margins": value])
  }
  public var markdownAutodetect: Bool { (root["editing"] as? [String: Any])?["markdownAutodetect"] as? Bool ?? true }
  public var caretHoverEffect: Bool { (root["editing"] as? [String: Any])?["caretHoverEffect"] as? Bool ?? true }
  public var smartQuotes: Bool { (root["editing"] as? [String: Any])?["smartQuotes"] as? Bool ?? false }
  public var markdownFormattedView: Bool {
    (root["editing"] as? [String: Any])?["markdownFormattedView"] as? Bool ?? false
  }
  /// Application default for hard-line reflow (`gq`/`gw`); buffers inherit it
  /// unless `:set textwidth` overrides them locally. Missing values use 80.
  public static let defaultTextWidth: UInt32 = 80
  public var textWidth: UInt32 {
    Self.validTextWidth((root["editing"] as? [String: Any])?["textWidth"]) ?? Self.defaultTextWidth
  }
  public var indentation: EVIndentationOptions {
    Self.editingOptions(root, key: "indentation") ?? EVIndentationOptions()
  }
  public var whitespacePresentation: EVWhitespacePresentationOptions {
    Self.editingOptions(root, key: "whitespacePresentation") ?? EVWhitespacePresentationOptions()
  }
  public var showStatusBar: Bool { (root["appearance"] as? [String: Any])?["showStatusBar"] as? Bool ?? true }
  public func showFormattingToolbar(for format: EVSourceFormat) -> Bool {
    (root["formattingToolbar"] as? [String: Bool])?[format.rawValue] ?? true
  }
  public func setShowFormattingToolbar(_ visible: Bool, for format: EVSourceFormat) throws {
    try update(section: "formattingToolbar", values: [format.rawValue: visible])
  }
  public var recentDocumentURLs: [URL] { Self.recentDocumentURLs(in: root) }
  public var documentWindowFrame: CGRect? {
    Self.documentWindowFrame(in: root)
  }
  public func setDocumentWindowFrame(_ frame: CGRect) throws {
    let fields: [String: Any] = ["x": frame.origin.x, "y": frame.origin.y,
      "width": frame.width, "height": frame.height]
    guard Self.documentWindowFrame(in: ["windows": ["documentFrame": fields]]) != nil else {
      throw invalid("Document window frame must have finite coordinates and positive dimensions")
    }
    try update(section: "windows", values: ["documentFrame": fields], notify: false)
  }
  /// Application resource discovery, independent of persisted preferences.
  public var bundledVimSyntaxDirectory: String {
    bundleResourceURL?.appendingPathComponent("vim/runtime/syntax", isDirectory: true).path ?? ""
  }
  public var codeFilenameAssociations: [EVCodeFilenameAssociation] {
    guard let entries = (root["code"] as? [String: Any])?["filenameAssociations"] as? [[String: Any]] else { return [] }
    return entries.compactMap { entry in
      guard let pattern = entry["pattern"] as? String, let language = entry["language"] as? String else { return nil }
      return EVCodeFilenameAssociation(pattern: pattern, language: language)
    }
  }
  public func codeFilenameAssociationsJSON() throws -> Data { try JSONEncoder().encode(codeFilenameAssociations) }

  public func setTheme(_ theme: EVTheme) throws {
    guard theme.isValid else { throw invalid("Invalid theme values") }
    let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(theme)) as! [String: Any]
    try updateActiveTheme(styleNames: []) { candidate in
      candidate["theme"] = Self.mergePreservingUnknown(candidate["theme"] as? [String: Any] ?? [:], value)
    }
  }
  public func setMarkdownAutodetect(_ enabled: Bool) throws { try update(section: "editing", values: ["markdownAutodetect": enabled]) }
  public func setCaretHoverEffect(_ enabled: Bool) throws { try update(section: "editing", values: ["caretHoverEffect": enabled]) }
  public func setSmartQuotes(_ enabled: Bool) throws { try update(section: "editing", values: ["smartQuotes": enabled]) }
  public func setMarkdownFormattedView(_ enabled: Bool) throws {
    // This default affects future opens; existing views retain their own state.
    try update(section: "editing", values: ["markdownFormattedView": enabled], notify: false)
  }
  public func setTextWidth(_ width: UInt32) throws {
    guard width > 0 else { throw invalid("Text width must be a positive whole number of columns") }
    try update(section: "editing", values: ["textWidth": NSNumber(value: width)])
  }
  public func setIndentation(_ options: EVIndentationOptions) throws {
    guard options.isValid else { throw invalid("Tab stop must be 1–1024, shift width 0–1024, and soft tab stop −1–1024") }
    let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(options))
    try update(section: "editing", values: ["indentation": object])
  }
  public func setWhitespacePresentation(_ options: EVWhitespacePresentationOptions) throws {
    guard (0...1024).contains(options.codeWrappedLineIndent) else { throw invalid("Wrapped line indent (Code) must be a whole number from 0 to 1024") }
    guard options.isValid else { throw invalid(EVListcharsSettings.validationError(options.visibleWhitespace.listchars) ?? "Invalid Visible whitespace style") }
    if let error = EVEditingPreferences.validateWhitespacePresentation?(options) { throw invalid(error) }
    var object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(options)) as! [String: Any]
    var visible = object["visibleWhitespace"] as! [String: Any]
    var style = visible["style"] as! [String: Any]
    // Explicit null clears known sparse declarations; unknown future fields are
    // still retained by the normal recursive configuration merge.
    for key in EVVisibleWhitespaceStyle.propertyNames where style[key] == nil { style[key] = NSNull() }
    visible["style"] = style
    object["visibleWhitespace"] = visible
    try update(section: "editing", values: ["whitespacePresentation": object])
  }

  /// Refreshes this store after another settings owner commits the same file.
  public func reloadFromDisk() throws {
    guard !reloadingFromDisk else { return }
    reloadingFromDisk = true
    defer { reloadingFromDisk = false }
    let file = directory.appendingPathComponent("config.json")
    guard manager.fileExists(atPath: file.path) else { return }
    do {
      let candidate = try Self.readObject(Data(contentsOf: file))
      try Self.validate(candidate)
      root = candidate
      lastError = nil
      writable = true
      // A malformed external theme must not prevent an otherwise valid
      // preference update from reaching its observers. Keep the last usable
      // theme; explicit theme reload still reports its validation failure.
      do { try reloadSelectedThemeFromSettings() }
      catch { lastError = error.localizedDescription }
    } catch { lastError = error.localizedDescription; throw error }
  }
  public func setShowStatusBar(_ enabled: Bool) throws { try update(section: "appearance", values: ["showStatusBar": enabled]) }
  public func setCodeFilenameAssociations(_ entries: [EVCodeFilenameAssociation]) throws {
    try update(section: "code", values: ["filenameAssociations": entries.map { ["pattern": $0.pattern, "language": $0.language] }])
  }

  public func recordRecentDocument(_ url: URL) throws {
    guard url.isFileURL, Self.isValidRecentDocumentPath(url.path) else {
      throw invalid("Recent documents must use absolute file paths of at most 16 KiB")
    }
    let canonical = EVDocumentIdentity.canonicalURL(url)
    try update { candidate in
      var recent = Self.recentDocumentURLs(in: candidate)
      recent.removeAll { EVDocumentIdentity.sameFile($0, canonical) }
      recent.insert(canonical, at: 0)
      let paths = Array(recent.prefix(10)).map(\.path)
      guard candidate["recentDocuments"] as? [String] != paths else { return false }
      candidate["recentDocuments"] = paths
      return true
    }
  }

  public func clearRecentDocuments() throws {
    try update { candidate in
      guard candidate["recentDocuments"] as? [String] != [] else { return false }
      candidate["recentDocuments"] = [String]()
      return true
    }
  }

  public func codeStyleSheet() throws -> Data? { try styleDefaults(named: "code") }
  public func saveCodeStyleSheet(_ data: Data, replacingInvalidFile: Bool = false) throws {
    try saveThemeStyles(data, named: "code", replacingInvalidFile: replacingInvalidFile)
  }
  public func styleDefaults(named name: String) throws -> Data? {
    guard Self.styleNames.contains(name) else { throw invalid("Unknown style format") }
    let value: Any?
    if let configured = (activeTheme["styles"] as? [String: Any])?[name] {
      value = configured
    } else {
      value = (try EVThemeFile.builtin()["styles"] as? [String: Any])?[name]
    }
    guard let value else { return nil }
    return try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
  }
  public func saveStyleDefaults(_ data: Data, named name: String) throws {
    try saveThemeStyles(data, named: name)
  }

  func update(section: String, values: [String: Any], notify: Bool = true) throws {
    try update(notify: notify) { candidate in
      candidate[section] = Self.mergePreservingUnknown(candidate[section] as? [String: Any] ?? [:], values)
      return true
    }
  }
  func update(notify: Bool = true, _ mutation: (inout [String: Any]) throws -> Bool) throws {
    guard writable else { throw invalid(lastError ?? "Configuration cannot be modified") }
    let url = directory.appendingPathComponent("config.json")
    do {
      var candidate = root
      // Re-read before committing so unrelated changes from another settings
      // control or application instance are retained. Invalid external changes
      // remain on disk and are reported by the settings window.
      let exists = manager.fileExists(atPath: url.path)
      if exists { candidate = try Self.readObject(Data(contentsOf: url)); try Self.validate(candidate) }
      let changed = try mutation(&candidate)
      // This removed preference never controls resource lookup. Retire old
      // values on the next settings write while retaining unrelated fields.
      if var code = candidate["code"] as? [String: Any] {
        code.removeValue(forKey: "vimSyntaxDirectory")
        candidate["code"] = code
      }
      try Self.validate(candidate)
      if changed || !exists { try write(candidate, to: url) }
      root = candidate
      lastError = nil
      if notify { NotificationCenter.default.post(name: .viemConfigurationDidChange, object: self) }
    }
    catch { lastError = error.localizedDescription; throw error }
  }
  func write(_ object: [String: Any], to url: URL) throws {
    let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed])
    guard data.count <= 4 * 1024 * 1024 else { throw invalid("Configuration must fit within 4 MiB") }
    try manager.createDirectory(at: directory, withIntermediateDirectories: true)
    try data.write(to: url, options: .atomic)
  }
  static func readObject(_ data: Data) throws -> [String: Any] {
    guard data.count <= 4 * 1024 * 1024,
          let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw invalid("Configuration must be a JSON object below 4 MiB") }
    return object
  }
  static func validateVersion(_ object: [String: Any]) throws {
    guard let version = object["version"] as? NSNumber, CFGetTypeID(version) != CFBooleanGetTypeID(), version.intValue == 1,
          version.doubleValue == 1 else { throw invalid("Unsupported configuration version; expected 1") }
  }
  static func validateStyleVersion(_ object: [String: Any], named name: String) throws {
    guard name == "code" else { try validateVersion(object); return }
    guard let version = object["version"] as? NSNumber,
          CFGetTypeID(version) != CFBooleanGetTypeID(),
          version.intValue == 3, version.doubleValue == 3
    else { throw invalid("Unsupported Code stylesheet version; expected 3") }
  }
  static func validate(_ object: [String: Any]) throws {
    try validateVersion(object)
    for key in ["selectedTheme", "selectedThemeFile"] {
      if let value = object[key], !(value is NSNull), !(value is String) {
        throw invalid("Selected theme must be a theme name and filename, or null for Default")
      }
    }
    if let raw = object["windows"] {
      guard let windows = raw as? [String: Any] else { throw invalid("Invalid window settings") }
      if windows["documentFrame"] != nil, documentWindowFrame(in: object) == nil {
        throw invalid("Document window frame must have finite coordinates and positive dimensions")
      }
    }
    if let raw = object["recentDocuments"] {
      guard let paths = raw as? [String], paths.count <= 10,
            paths.allSatisfy(isValidRecentDocumentPath) else {
        throw invalid("Recent documents must be an array of at most 10 absolute paths, each at most 16 KiB")
      }
    }
    if let raw = object["view"] {
      guard let fields = raw as? [String: Any] else { throw invalid("Invalid View settings") }
      if let value = fields["margins"] {
        let margins = try JSONDecoder().decode(EVViewMargins.self, from: JSONSerialization.data(withJSONObject: value))
        guard margins.isValid else { throw invalid("View margins must be between 0 and 1000 pixels") }
      }
    }
    for (section, key) in [("editing", "caretHoverEffect"), ("editing", "markdownAutodetect"), ("editing", "smartQuotes"), ("editing", "markdownFormattedView"), ("appearance", "showStatusBar")] {
      if let raw = object[section] {
        guard let fields = raw as? [String: Any] else { throw invalid("Invalid \(section) settings") }
        if let value = fields[key] {
          guard let boolean = value as? NSNumber, CFGetTypeID(boolean) == CFBooleanGetTypeID() else { throw invalid("\(key) must be Boolean") }
        }
      }
    }
    if let raw = object["formattingToolbar"] {
      guard let fields = raw as? [String: Any], fields.values.allSatisfy({ value in
        guard let boolean = value as? NSNumber else { return false }
        return CFGetTypeID(boolean) == CFBooleanGetTypeID()
      }) else { throw invalid("Formatting toolbar visibility must be Boolean per format") }
    }
    if let value = (object["editing"] as? [String: Any])?["textWidth"], validTextWidth(value) == nil {
      throw invalid("textWidth must be a positive whole number of columns up to 4294967295")
    }
    if let editing = object["editing"] as? [String: Any] {
      if let raw = editing["indentation"] {
        let value = try JSONDecoder().decode(EVIndentationOptions.self, from: JSONSerialization.data(withJSONObject: raw, options: .fragmentsAllowed))
        guard value.isValid else { throw invalid("Invalid indentation defaults") }
      }
      if let raw = editing["whitespacePresentation"] {
        let value = try JSONDecoder().decode(EVWhitespacePresentationOptions.self, from: JSONSerialization.data(withJSONObject: raw, options: .fragmentsAllowed))
        guard (0...1024).contains(value.codeWrappedLineIndent) else { throw invalid("Wrapped line indent (Code) must be a whole number from 0 to 1024") }
        guard value.isValid else { throw invalid(EVListcharsSettings.validationError(value.visibleWhitespace.listchars) ?? "Invalid Visible whitespace style") }
      }
    }
    if let raw = object["code"] {
      guard let fields = raw as? [String: Any] else { throw invalid("Invalid Code settings") }
      if let rawAssociations = fields["filenameAssociations"] {
        guard let entries = rawAssociations as? [[String: Any]], entries.count <= 256 else {
          throw invalid("Code filename associations must be an array of at most 256 entries")
        }
        var normalized: [EVCodeFilenameAssociation] = []
        for entry in entries {
          guard let pattern = entry["pattern"] as? String, !pattern.isEmpty, pattern.utf8.count <= 256, !pattern.contains("\0"),
                let language = entry["language"] as? String, !language.isEmpty, language.utf8.count <= 128,
                language.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [95, 43, 46, 35, 45].contains($0) }) else {
            throw invalid("Each Code filename association requires a pattern of 1–256 bytes and a language name of 1–128 ASCII letters, digits, or _+.#- characters")
          }
          normalized.append(EVCodeFilenameAssociation(pattern: pattern, language: language))
        }
        guard try JSONEncoder().encode(normalized).count <= 128 * 1024 else {
          throw invalid("Encoded Code filename associations must fit within 128 KiB")
        }
      }
    }
  }
  /// A positive unsigned 32-bit integer. Booleans, fractions, zero, negative
  /// values, overflow, and non-numbers are rejected.
  private static func validTextWidth(_ value: Any?) -> UInt32? {
    guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { return nil }
    let double = number.doubleValue
    guard double.isFinite, double >= 1, double <= Double(UInt32.max), double == double.rounded(.towardZero) else { return nil }
    return UInt32(exactly: double)
  }
  private static func documentWindowFrame(in object: [String: Any]) -> CGRect? {
    guard let fields = (object["windows"] as? [String: Any])?["documentFrame"] as? [String: Any] else { return nil }
    var values: [Double] = []
    for key in ["x", "y", "width", "height"] {
      guard let number = fields[key] as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
            number.doubleValue.isFinite else { return nil }
      values.append(number.doubleValue)
    }
    guard values[2] > 0, values[3] > 0,
          (values[0] + values[2]).isFinite, (values[1] + values[3]).isFinite else { return nil }
    return CGRect(x: values[0], y: values[1], width: values[2], height: values[3])
  }
  private static func editingOptions<T: Decodable>(_ root: [String: Any], key: String) -> T? {
    guard let object = (root["editing"] as? [String: Any])?[key],
          let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
    return try? JSONDecoder().decode(T.self, from: data)
  }
  private static func isValidRecentDocumentPath(_ path: String) -> Bool {
    path.hasPrefix("/") && !path.contains("\0") && path.utf8.count <= 16_384
  }
  private static func recentDocumentURLs(in object: [String: Any]) -> [URL] {
    var result: [URL] = []
    for path in object["recentDocuments"] as? [String] ?? [] {
      let url = EVDocumentIdentity.canonicalURL(URL(fileURLWithPath: path))
      if !result.contains(where: { EVDocumentIdentity.sameFile($0, url) }) {
        result.append(url)
      }
    }
    return result
  }
  static func mergePreservingUnknown(_ old: [String: Any], _ new: [String: Any]) -> [String: Any] {
    var result = old
    for (key, value) in new {
      if let a = old[key] as? [String: Any], let b = value as? [String: Any] { result[key] = mergePreservingUnknown(a, b) }
      else if ["block_styles", "character_styles"].contains(key), let a = old[key] as? [[String: Any]], let b = value as? [[String: Any]] {
        result[key] = b.map { entry in
          guard let id = entry["id"] as? String, let previous = a.first(where: { $0["id"] as? String == id }) else { return entry }
          return mergePreservingUnknown(previous, entry)
        }
      } else { result[key] = value }
    }
    return result
  }
  static func invalid(_ message: String) -> NSError { NSError(domain: "Viem.Configuration", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
  func invalid(_ message: String) -> NSError { Self.invalid(message) }
}

extension EVSourceFormat {
  public var defaultStyleName: String {
    switch self {
    case .plainText: "text"
    case .markdown, .markdownSource: "markdown"
    case .code: "code"
    }
  }
}
