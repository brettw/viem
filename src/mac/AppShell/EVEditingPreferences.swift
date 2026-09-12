import Foundation

extension Notification.Name {
  public static let viemEditingPreferencesDidChange = Notification.Name(
    "com.viem.editing-preferences.did-change")
}

/// Application preferences configure portable input assistance. They never
/// become document declarations or rewrite existing source when changed.
@MainActor
public final class EVEditingPreferences {
  public static let shared = EVEditingPreferences()
  private let configuration: EVConfigurationStore
  public var lastError: String? { configuration.lastError }
  private let center: NotificationCenter
  public private(set) var smartQuotes: Bool
  /// Application default `textwidth` in columns for `gq`/`gw` reflow.
  public private(set) var textWidth: UInt32

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    smartQuotes = configuration.smartQuotes
    textWidth = configuration.textWidth
  }

  /// Rejects zero without changing the prior value. Returns whether the
  /// requested width is now the effective default.
  @discardableResult
  public func setTextWidth(_ width: UInt32) -> Bool {
    guard width != textWidth else { return true }
    guard width > 0 else { return false }
    do { try configuration.setTextWidth(width) } catch { return false }
    textWidth = width
    center.post(name: .viemEditingPreferencesDidChange, object: self)
    return true
  }

  public func setSmartQuotes(_ enabled: Bool) {
    guard enabled != smartQuotes else { return }
    do { try configuration.setSmartQuotes(enabled) } catch { return }
    smartQuotes = enabled
    center.post(name: .viemEditingPreferencesDidChange, object: self)
  }
}
