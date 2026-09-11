import Foundation
import CoreFoundation

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
  public private(set) var lastError: String?
  private var root: [String: Any] = ["version": 1]
  private var writable = true
  private let manager: FileManager

  public init(directory: URL? = nil, legacyDefaults: UserDefaults? = nil,
              manager: FileManager = .default) {
    self.manager = manager
    let overriddenDirectory = ProcessInfo.processInfo.environment["VIEM_CONFIG_DIR"]
    // Injected directories are isolated (tests, previews, portable profiles):
    // never consume the real application's legacy preferences there.
    let legacy = legacyDefaults ?? (directory == nil && overriddenDirectory == nil ? UserDefaults.standard : nil)
    self.directory = directory ?? overriddenDirectory.map {
      URL(fileURLWithPath: $0, isDirectory: true)
    } ?? manager.homeDirectoryForCurrentUser.appendingPathComponent(".viem", isDirectory: true)
    let file = self.directory.appendingPathComponent("config.json")
    do {
      if manager.fileExists(atPath: file.path) {
        root = try Self.readObject(Data(contentsOf: file))
        try Self.validate(root)
      } else {
        if let data = legacy?.data(forKey: "EVApplicationTheme.v1"),
           let theme = try? JSONDecoder().decode(EVTheme.self, from: data), theme.isValid {
          root["theme"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(theme))
        }
        if let value = legacy?.object(forKey: "EVEditing.SmartQuotes.v1") as? Bool {
          root["editing"] = ["smartQuotes": value]
        }
        if let value = legacy?.object(forKey: "EVShowStatusBar") as? Bool {
          root["appearance"] = ["showStatusBar": value]
        }
        if root.count > 1 {
          try write(root, to: file)
          ["EVApplicationTheme.v1", "EVEditing.SmartQuotes.v1", "EVShowStatusBar"].forEach {
            legacy?.removeObject(forKey: $0)
          }
        }
      }
    } catch {
      lastError = error.localizedDescription
      writable = false
      root = ["version": 1]
    }
  }

  public var theme: EVTheme {
    guard let value = root["theme"], let data = try? JSONSerialization.data(withJSONObject: value),
          let result = try? JSONDecoder().decode(EVTheme.self, from: data), result.isValid else { return .paper }
    return result
  }
  public var smartQuotes: Bool { (root["editing"] as? [String: Any])?["smartQuotes"] as? Bool ?? false }
  public var showStatusBar: Bool { (root["appearance"] as? [String: Any])?["showStatusBar"] as? Bool ?? true }
  public var vimSyntaxDirectory: String {
    (root["code"] as? [String: Any])?["vimSyntaxDirectory"] as? String ?? EVCodePreferences.defaultVimSyntaxDirectory
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
    try update(section: "theme", values: value)
  }
  public func setSmartQuotes(_ enabled: Bool) throws { try update(section: "editing", values: ["smartQuotes": enabled]) }
  public func setShowStatusBar(_ enabled: Bool) throws { try update(section: "appearance", values: ["showStatusBar": enabled]) }
  public func setVimSyntaxDirectory(_ path: String) throws {
    try update(section: "code", values: ["vimSyntaxDirectory": path])
  }
  public func setCodeFilenameAssociations(_ entries: [EVCodeFilenameAssociation]) throws {
    try update(section: "code", values: ["filenameAssociations": entries.map { ["pattern": $0.pattern, "language": $0.language] }])
  }

  public func codeStyleSheet() throws -> Data? { try styleDefaults(named: "code") }
  public func saveCodeStyleSheet(_ data: Data, replacingInvalidFile: Bool = false) throws {
    let url = try styleURL("code")
    let object = try Self.readObject(data)
    try Self.validateStyleVersion(object, named: "code")
    if !replacingInvalidFile, manager.fileExists(atPath: url.path) {
      try Self.validateStyleVersion(Self.readObject(Data(contentsOf: url)), named: "code")
    }
    // The core exports the complete sparse authority, including explicit
    // suppression of built-ins. Merging removed declarations from an older
    // export would undo the user's Clear, Rename, Delete, or Undo action.
    try write(object, to: url)
  }

  public func styleDefaults(named name: String) throws -> Data? {
    let url = try styleURL(name)
    guard manager.fileExists(atPath: url.path) else { return nil }
    let data = try Data(contentsOf: url)
    guard data.count <= 4 * 1024 * 1024 else { throw invalid("Style defaults exceed 4 MiB") }
    let object = try Self.readObject(data)
    try Self.validateStyleVersion(object, named: name)
    return data
  }

  public func saveStyleDefaults(_ data: Data, named name: String) throws {
    if name == "code" {
      try saveCodeStyleSheet(data)
      return
    }
    let url = try styleURL(name)
    var object = try Self.readObject(data)
    try Self.validateVersion(object)
    if manager.fileExists(atPath: url.path) {
      let old = try Self.readObject(Data(contentsOf: url))
      try Self.validateVersion(old)
      object = Self.mergePreservingUnknown(old, object)
    }
    try write(object, to: url)
  }

  private func styleURL(_ name: String) throws -> URL {
    guard ["text", "html", "markdown", "rtf", "code"].contains(name) else { throw invalid("Unknown style format") }
    return directory.appendingPathComponent("\(name)_style.json")
  }
  private func update(section: String, values: [String: Any]) throws {
    guard writable else { throw invalid(lastError ?? "Configuration cannot be modified") }
    let url = directory.appendingPathComponent("config.json")
    do {
      var candidate = root
      // Re-read before committing so unrelated changes from another settings
      // control or application instance are retained. Invalid external changes
      // remain on disk and are reported by the settings window.
      if manager.fileExists(atPath: url.path) { candidate = try Self.readObject(Data(contentsOf: url)); try Self.validate(candidate) }
      candidate[section] = Self.mergePreservingUnknown(candidate[section] as? [String: Any] ?? [:], values)
      try Self.validate(candidate)
      try write(candidate, to: url)
      root = candidate
      lastError = nil
    }
    catch { lastError = error.localizedDescription; throw error }
  }
  private func write(_ object: [String: Any], to url: URL) throws {
    let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed])
    try manager.createDirectory(at: directory, withIntermediateDirectories: true)
    try data.write(to: url, options: .atomic)
  }
  private static func readObject(_ data: Data) throws -> [String: Any] {
    guard data.count <= 4 * 1024 * 1024,
          let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw invalid("Configuration must be a JSON object below 4 MiB") }
    return object
  }
  private static func validateVersion(_ object: [String: Any]) throws {
    guard let version = object["version"] as? NSNumber, CFGetTypeID(version) != CFBooleanGetTypeID(), version.intValue == 1,
          version.doubleValue == 1 else { throw invalid("Unsupported configuration version; expected 1") }
  }
  private static func validateStyleVersion(_ object: [String: Any], named name: String) throws {
    guard name == "code" else { try validateVersion(object); return }
    // The core migrates Code's copied version-1 declarations to the linked
    // version-2 stylesheet. Other settings and format defaults remain at 1.
    guard let version = object["version"] as? NSNumber,
          CFGetTypeID(version) != CFBooleanGetTypeID(),
          [1, 2].contains(version.intValue),
          version.doubleValue == Double(version.intValue)
    else { throw invalid("Unsupported Code stylesheet version; expected 1 or 2") }
  }
  private static func validate(_ object: [String: Any]) throws {
    try validateVersion(object)
    if let value = object["theme"] {
      let data = try JSONSerialization.data(withJSONObject: value)
      let theme = try JSONDecoder().decode(EVTheme.self, from: data)
      guard theme.isValid else { throw invalid("Invalid theme values") }
    }
    for (section, key) in [("editing", "smartQuotes"), ("appearance", "showStatusBar")] {
      if let raw = object[section] {
        guard let fields = raw as? [String: Any] else { throw invalid("Invalid \(section) settings") }
        if let value = fields[key] {
          guard let boolean = value as? NSNumber, CFGetTypeID(boolean) == CFBooleanGetTypeID() else { throw invalid("\(key) must be Boolean") }
        }
      }
    }
    if let raw = object["code"] {
      guard let fields = raw as? [String: Any] else { throw invalid("Invalid Code settings") }
      if let rawPath = fields["vimSyntaxDirectory"] {
        guard let path = rawPath as? String, !path.contains("\0"), path.utf8.count <= 16_384 else {
          throw invalid("Vim syntax directory must be a path of at most 16 KiB")
        }
      }
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
  private static func mergePreservingUnknown(_ old: [String: Any], _ new: [String: Any]) -> [String: Any] {
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
  private static func invalid(_ message: String) -> NSError { NSError(domain: "Viem.Configuration", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
  private func invalid(_ message: String) -> NSError { Self.invalid(message) }
}

extension EVSourceFormat {
  public var defaultStyleName: String {
    switch self {
    case .plainText: "text"
    case .markdown, .markdownSource: "markdown"
    case .html, .htmlSource: "html"
    case .rtf: "rtf"
    case .code: "code"
    }
  }
}
