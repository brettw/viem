import Foundation
import CViemCore

public struct EVThemeChoice: Equatable {
  public let name: String
  public let url: URL
  public var fileName: String { url.lastPathComponent }
}

/// Theme structure, built-in values, and name rules are shared with Windows in
/// the portable core. Native code owns only profile discovery and file I/O.
@MainActor
enum EVThemeFile {
  static let maximumSize = 20 * 1024 * 1024
  private static var encodedBuiltins: [Bool: Data] = [:]

  static func builtin(paper: Bool = false) throws -> [String: Any] {
    var required: UInt64 = 0
    let preset: UInt32 = paper ? 1 : 0
    let status = viem_theme_default_json(preset, nil, 0, &required)
    guard [UInt32(VIEM_STATUS_OK), UInt32(VIEM_STATUS_BUFFER_TOO_SMALL)].contains(status),
          required <= maximumSize else { throw EVConfigurationStore.invalid("Cannot read the built-in theme") }
    var data = Data(count: Int(required))
    let result = data.withUnsafeMutableBytes { raw in
      viem_theme_default_json(preset, raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count), &required)
    }
    guard result == UInt32(VIEM_STATUS_OK) else { throw EVConfigurationStore.invalid("Cannot read the built-in theme") }
    return try decode(data)
  }

  static func decode(_ data: Data) throws -> [String: Any] {
    guard data.count <= maximumSize else { throw EVConfigurationStore.invalid("Theme exceeds 20 MiB") }
    let status = data.withUnsafeBytes { raw in
      viem_theme_validate_json(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
    }
    guard status == UInt32(VIEM_STATUS_OK),
          let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw EVConfigurationStore.invalid("The theme contains invalid settings or an unsupported version")
    }
    return result
  }

  static func encode(_ object: [String: Any]) throws -> Data {
    let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
    _ = try decode(data)
    return data
  }

  fileprivate static func encodedBuiltin(paper: Bool) throws -> Data {
    if let data = encodedBuiltins[paper] { return data }
    // These two core presets are immutable for the life of the executable.
    // Cache their validated bytes rather than sharing mutable theme objects;
    // bundled and user-authored files still come from the filesystem.
    let data = try encode(builtin(paper: paper))
    encodedBuiltins[paper] = data
    return data
  }

  static func read(_ file: URL) throws -> Data {
    let handle = try FileHandle(forReadingFrom: file)
    defer { try? handle.close() }
    let data = try handle.read(upToCount: maximumSize + 1) ?? Data()
    guard data.count <= maximumSize else { throw EVConfigurationStore.invalid("Theme exceeds 20 MiB") }
    return data
  }
}

extension EVConfigurationStore {
  static let styleNames = ["text", "markdown", "code"]

  public var themesDirectory: URL { directory.appendingPathComponent("themes", isDirectory: true) }
  public var currentThemeName: String? { activeThemeName }
  public var selectedThemeURL: URL? { activeThemeFile }
  public var currentThemeFileName: String? { activeThemeFile?.lastPathComponent }

  public func ensureCurrentThemeExists() throws {
    if let file = activeThemeFile, !manager.fileExists(atPath: file.path) {
      let fallback = try EVThemeFile.builtin()
      var persistenceError: Error?
      do { try persistThemeSelection(nil) }
      catch { persistenceError = error }
      installTheme(fallback, name: nil, file: nil, diskData: nil, selectionChanged: true)
      if let persistenceError {
        lastError = persistenceError.localizedDescription
        throw persistenceError
      }
    }
  }

  /// The filesystem is the catalogue. No separate index can become stale when
  /// a user adds, renames, or removes a theme outside the application.
  private var themeFiles: [(name: String, url: URL)] {
    let files = (try? manager.contentsOfDirectory(at: themesDirectory,
      includingPropertiesForKeys: [.isRegularFileKey])) ?? []
    return files.compactMap { file -> (name: String, url: URL)? in
      guard (try? file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else { return nil }
      let filename = file.lastPathComponent
      let name = filename.lowercased().hasSuffix(".json") ? String(filename.dropLast(5)) : filename
      return (name, file)
    }.sorted {
      let first = $0.name.lowercased(), second = $1.name.lowercased()
      return first == second ? $0.name < $1.name : first < second
    }
  }

  public var availableThemeNames: [String] { themeFiles.map(\.name) }
  public var availableThemes: [EVThemeChoice] { themeFiles.map { EVThemeChoice(name: $0.name, url: $0.url) } }

  public static func validateThemeName(_ name: String) throws {
    let data = Data(name.utf8)
    let status = data.withUnsafeBytes { raw in
      viem_theme_validate_name(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
    }
    guard status == UInt32(VIEM_STATUS_OK) else {
      throw invalid("Use 1–32 filename-safe characters. Avoid slashes, control characters, trailing dots or spaces, reserved device names, and Default.")
    }
  }

  public func selectTheme(named name: String?, fileName: String? = nil) throws {
    let entry = name.flatMap { name in themeFiles.first {
      $0.name == name && (fileName == nil || $0.url.lastPathComponent == fileName)
    } }
    let data = try entry.map { try EVThemeFile.read($0.url) }
    let candidate = try data.map(EVThemeFile.decode) ?? EVThemeFile.builtin()
    try persistThemeSelection(entry?.name, fileName: entry?.url.lastPathComponent)
    installTheme(candidate, name: entry?.name, file: entry?.url, diskData: data, selectionChanged: true)
  }

  /// Capture the live aggregate, including unsaved edits to Default, before
  /// selecting the new file. Neither a preset nor an older disk snapshot is
  /// substituted for the current values.
  public func createTheme(named name: String) throws {
    try Self.validateThemeName(name)
    guard !availableThemeNames.contains(where: { $0.caseInsensitiveCompare(name) == .orderedSame }) else {
      throw invalid("A theme with that name already exists")
    }
    let data = try EVThemeFile.encode(activeTheme)
    let file = themesDirectory.appendingPathComponent(name + ".json")
    try manager.createDirectory(at: themesDirectory, withIntermediateDirectories: true)
    try data.write(to: file, options: .withoutOverwriting)
    do { try persistThemeSelection(name) }
    catch { try? manager.removeItem(at: file); throw error }
    installTheme(activeTheme, name: name, file: file, diskData: data, selectionChanged: true)
  }

  public func deleteCurrentTheme() throws {
    guard let file = activeThemeFile else { return }
    let candidate = try EVThemeFile.builtin()
    let data = try EVThemeFile.read(file)
    try manager.removeItem(at: file)
    do { try persistThemeSelection(nil) }
    catch { try? data.write(to: file, options: .withoutOverwriting); throw error }
    installTheme(candidate, name: nil, file: nil, diskData: nil, selectionChanged: true)
  }

  public func reloadCurrentTheme() throws {
    if let file = activeThemeFile, !manager.fileExists(atPath: file.path) {
      try ensureCurrentThemeExists()
      return
    }
    guard let file = activeThemeFile else {
      try selectTheme(named: nil)
      return
    }
    try applyThemeFileData(EVThemeFile.read(file))
  }

  /// Installs a monitor's already-read snapshot without writing it back.
  public func applyThemeFileData(_ data: Data) throws {
    let candidate = try EVThemeFile.decode(data)
    installTheme(candidate, name: activeThemeName, file: activeThemeFile,
      diskData: data, selectionChanged: false)
  }

  func reloadSelectedThemeFromSettings() throws {
    let selected = root["selectedTheme"] as? String
    let filename = root["selectedThemeFile"] as? String
    let entry = themeFiles.first { $0.name == selected && (filename == nil || $0.url.lastPathComponent == filename) }
    let data = try entry.map { try EVThemeFile.read($0.url) }
    // Preferences observers reread config.json after a settings notification.
    // Do not republish an unchanged theme recursively, or discard Default's
    // in-memory edits while an unrelated preference is being refreshed.
    if entry?.name == activeThemeName, entry?.url == activeThemeFile, data == activeThemeDiskData { return }
    let candidate = try data.map(EVThemeFile.decode) ?? EVThemeFile.builtin()
    installTheme(candidate, name: entry?.name, file: entry?.url, diskData: data, selectionChanged: true)
  }

  func initializeThemes(newProfile: Bool) throws {
    let hadThemes = manager.fileExists(atPath: themesDirectory.path)
    try manager.createDirectory(at: themesDirectory, withIntermediateDirectories: true)
    if newProfile && !hadThemes {
      for name in ["Paper", "Midnight", "Midnight Mono", "Midnight Proportional", "Typewriter"] {
        let bundled = bundleResourceURL?.appendingPathComponent("themes/\(name).json")
        let data: Data
        if let bundled, manager.fileExists(atPath: bundled.path) {
          data = try EVThemeFile.read(bundled)
          _ = try EVThemeFile.decode(data)
        } else {
          // These packaged variants have no independent emergency fallback.
          if name != "Paper" && name != "Midnight" { continue }
          // Development/test executables and damaged bundles still have a
          // complete usable default without depending on resource files.
          data = try EVThemeFile.encodedBuiltin(paper: name == "Paper")
        }
        try data.write(to: themesDirectory.appendingPathComponent(name + ".json"), options: .withoutOverwriting)
      }
    }

    if root["selectedTheme"] == nil && newProfile && !hadThemes {
      try persistThemeSelection("Midnight")
    }
    let selected = root["selectedTheme"] as? String
    let filename = root["selectedThemeFile"] as? String
    if let entry = themeFiles.first(where: { $0.name == selected && (filename == nil || $0.url.lastPathComponent == filename) }) {
      do {
        let data = try EVThemeFile.read(entry.url)
        activeTheme = try EVThemeFile.decode(data)
        activeThemeName = entry.name
        activeThemeFile = entry.url
        activeThemeDiskData = data
      } catch {
        // A broken theme must not disable unrelated application preferences or
        // overwrite the user's file. Default remains editable in memory.
        lastError = "Unable to load \(entry.name): \(error.localizedDescription). Default is active."
      }
    }
  }

  func saveThemeStyles(_ data: Data, named name: String, replacingInvalidFile: Bool = false) throws {
    guard Self.styleNames.contains(name) else { throw invalid("Unknown style format") }
    let object = try Self.readObject(data)
    try Self.validateStyleVersion(object, named: name)
    try updateActiveTheme(styleNames: [name], replacingInvalidFile: replacingInvalidFile) { candidate in
      var styles = candidate["styles"] as? [String: Any] ?? [:]
      // Code export includes suppression records and is a complete authority.
      // For other sheets retain future fields on surviving definitions only.
      if name == "code" {
        var complete = styles[name] as? [String: Any] ?? [:]
        for key in ["version", "block_styles", "character_styles", "suppressed_character_ids"] { complete.removeValue(forKey: key) }
        complete.merge(object) { _, new in new }
        styles[name] = complete
      } else {
        styles[name] = Self.mergeThemeStyleSheet(styles[name] as? [String: Any] ?? [:], object)
      }
      candidate["styles"] = styles
    }
  }

  /// Preserve extension keys at schema record boundaries. Property values are
  /// complete values: recursively merging a feature map or tagged enum would
  /// restore cleared entries or combine two mutually exclusive variants.
  private static func mergeThemeStyleSheet(_ old: [String: Any], _ new: [String: Any]) -> [String: Any] {
    var result = old
    for (key, value) in new {
      guard ["block_styles", "character_styles"].contains(key),
            let entries = value as? [[String: Any]] else { result[key] = value; continue }
      let previous = old[key] as? [[String: Any]] ?? []
      result[key] = entries.map { entry -> [String: Any] in
        guard let id = entry["id"] as? String,
              var merged = previous.first(where: { $0["id"] as? String == id }) else { return entry }
        for (field, value) in entry {
          if ["character", "block", "properties"].contains(field), let properties = value as? [String: Any] {
            var combined = merged[field] as? [String: Any] ?? [:]
            combined.merge(properties) { _, replacement in replacement }
            merged[field] = combined
          } else { merged[field] = value }
        }
        return merged
      }
    }
    return result
  }

  func updateActiveTheme(styleNames: [String], replacingInvalidFile: Bool = false,
                         _ mutation: (inout [String: Any]) throws -> Void) throws {
    do {
      var candidate = activeTheme
      var changedStyles = styleNames
      if let file = activeThemeFile {
        if !manager.fileExists(atPath: file.path) {
          try ensureCurrentThemeExists()
          candidate = activeTheme
        } else if !replacingInvalidFile {
          let diskData = try EVThemeFile.read(file)
          candidate = try EVThemeFile.decode(diskData)
          if diskData != activeThemeDiskData { changedStyles = Self.styleNames }
        }
      }
      try mutation(&candidate)
      let data = try EVThemeFile.encode(candidate)
      if let file = activeThemeFile {
        try data.write(to: file, options: .atomic)
        activeThemeDiskData = data
      }
      activeTheme = candidate
      lastError = nil
      notifyThemeChange(selectionChanged: false, styleNames: changedStyles)
    } catch { lastError = error.localizedDescription; throw error }
  }

  private func persistThemeSelection(_ name: String?, fileName: String? = nil) throws {
    try update(notify: false) { candidate in
      candidate["selectedTheme"] = name as Any? ?? NSNull()
      candidate["selectedThemeFile"] = name.map { fileName ?? ($0 + ".json") } as Any? ?? NSNull()
      return true
    }
  }

  private func installTheme(_ value: [String: Any], name: String?, file: URL?, diskData: Data?, selectionChanged: Bool) {
    activeTheme = value
    activeThemeName = name
    activeThemeFile = file
    activeThemeDiskData = diskData
    lastError = nil
    notifyThemeChange(selectionChanged: selectionChanged, styleNames: Self.styleNames)
  }

  private func notifyThemeChange(selectionChanged: Bool, styleNames: [String]) {
    NotificationCenter.default.post(name: .viemThemeDidChange, object: self,
      userInfo: ["selectionChanged": selectionChanged, "styleNames": styleNames])
    NotificationCenter.default.post(name: .viemConfigurationDidChange, object: self)
  }
}
