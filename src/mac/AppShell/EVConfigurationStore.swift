import Foundation
import CoreFoundation

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

  public func setTheme(_ theme: EVTheme) throws {
    guard theme.isValid else { throw invalid("Invalid theme values") }
    let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(theme)) as! [String: Any]
    try update(section: "theme", values: value)
  }
  public func setSmartQuotes(_ enabled: Bool) throws { try update(section: "editing", values: ["smartQuotes": enabled]) }
  public func setShowStatusBar(_ enabled: Bool) throws { try update(section: "appearance", values: ["showStatusBar": enabled]) }

  public func styleDefaults(named name: String) throws -> Data? {
    let url = try styleURL(name)
    guard manager.fileExists(atPath: url.path) else { return nil }
    let data = try Data(contentsOf: url)
    guard data.count <= 4 * 1024 * 1024 else { throw invalid("Style defaults exceed 4 MiB") }
    let object = try Self.readObject(data)
    try Self.validateVersion(object)
    return data
  }

  public func saveStyleDefaults(_ data: Data, named name: String) throws {
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
    guard ["text", "html", "markdown", "rtf"].contains(name) else { throw invalid("Unknown style format") }
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
    }
  }
}
