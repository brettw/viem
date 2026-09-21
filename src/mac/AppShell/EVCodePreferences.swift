import Foundation

extension Notification.Name {
  public static let viemCodeDiagnosticsDidChange = Notification.Name("com.viem.code-diagnostics.did-change")
}

/// Code style settings and diagnostics for bundled syntax analysis.
@MainActor
public final class EVCodePreferences {
  public static let shared = EVCodePreferences()
  public let configuration: EVConfigurationStore
  public private(set) var loadDiagnostics: [String] = []
  private var diagnosticsBySource: [String: [String]] = [:]
  public var lastError: String? { configuration.lastError }
  private let center: NotificationCenter

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    self.configuration = configuration ?? .shared
    self.center = center
  }
  public func reportLoadDiagnostics(_ diagnostics: [String], source: String = "application") {
    if diagnostics.isEmpty { diagnosticsBySource.removeValue(forKey: source) }
    else { diagnosticsBySource[source] = diagnostics }
    let combined = Array(Set(diagnosticsBySource.values.flatMap { $0 })).sorted()
    guard combined != loadDiagnostics else { return }
    loadDiagnostics = combined
    center.post(name: .viemCodeDiagnosticsDidChange, object: self)
  }

  public var bundledSyntaxDiagnostic: String? {
    let path = configuration.bundledVimSyntaxDirectory
    var isDirectory: ObjCBool = false
    guard !path.isEmpty, FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory),
          isDirectory.boolValue, FileManager.default.isReadableFile(atPath: path) else {
      return "Bundled Vim syntax files are unavailable. Tree-sitter highlighting remains available for supported languages."
    }
    return nil
  }
}
