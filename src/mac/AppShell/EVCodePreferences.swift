import Foundation

extension Notification.Name {
  public static let viemCodePreferencesDidChange = Notification.Name("com.viem.code-preferences.did-change")
  public static let viemCodeDiagnosticsDidChange = Notification.Name("com.viem.code-diagnostics.did-change")
}

/// Native persistence and resource paths for portable Code analysis.
@MainActor
public final class EVCodePreferences {
  public static let defaultVimSyntaxDirectory = "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax"
  public static let shared = EVCodePreferences()
  public static var editStyles: (@MainActor (EVConfigurationStore) -> Void)?
  public let configuration: EVConfigurationStore
  public private(set) var vimSyntaxDirectory: String
  public private(set) var loadDiagnostics: [String] = []
  private var diagnosticsBySource: [String: [String]] = [:]
  public var lastError: String? { configuration.lastError }
  private let center: NotificationCenter

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    self.configuration = configuration ?? .shared
    self.center = center
    vimSyntaxDirectory = self.configuration.vimSyntaxDirectory
  }

  @discardableResult
  public func setVimSyntaxDirectory(_ path: String) -> Bool {
    guard path != vimSyntaxDirectory else { return true }
    do { try configuration.setVimSyntaxDirectory(path) } catch { return false }
    vimSyntaxDirectory = path
    center.post(name: .viemCodePreferencesDidChange, object: self)
    return true
  }

  public func restoreDefaultDirectory() { _ = setVimSyntaxDirectory(Self.defaultVimSyntaxDirectory) }
  public func openStyles() { Self.editStyles?(configuration) }
  public func reportLoadDiagnostics(_ diagnostics: [String], source: String = "application") {
    if diagnostics.isEmpty { diagnosticsBySource.removeValue(forKey: source) }
    else { diagnosticsBySource[source] = diagnostics }
    let combined = Array(Set(diagnosticsBySource.values.flatMap { $0 })).sorted()
    guard combined != loadDiagnostics else { return }
    loadDiagnostics = combined
    center.post(name: .viemCodeDiagnosticsDidChange, object: self)
  }

  public var directoryDiagnostic: String? {
    let path = (vimSyntaxDirectory as NSString).expandingTildeInPath
    var isDirectory: ObjCBool = false
    guard !path.isEmpty, FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory),
          isDirectory.boolValue, FileManager.default.isReadableFile(atPath: path) else {
      return "The Vim syntax directory is unavailable. Bundled syntax highlighting remains available."
    }
    return nil
  }
}
