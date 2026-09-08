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

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    smartQuotes = configuration.smartQuotes
  }

  public func setSmartQuotes(_ enabled: Bool) {
    guard enabled != smartQuotes else { return }
    do { try configuration.setSmartQuotes(enabled) } catch { return }
    smartQuotes = enabled
    center.post(name: .viemEditingPreferencesDidChange, object: self)
  }
}
