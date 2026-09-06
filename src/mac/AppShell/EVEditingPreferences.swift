import Foundation

extension Notification.Name {
  public static let evimEditingPreferencesDidChange = Notification.Name(
    "com.evim.editing-preferences.did-change")
}

/// Application preferences configure portable input assistance. They never
/// become document declarations or rewrite existing source when changed.
@MainActor
public final class EVEditingPreferences {
  public static let shared = EVEditingPreferences()
  private static let smartQuotesKey = "EVEditing.SmartQuotes.v1"
  private let defaults: UserDefaults
  private let center: NotificationCenter
  public private(set) var smartQuotes: Bool

  public init(defaults: UserDefaults = .standard, center: NotificationCenter = .default) {
    self.defaults = defaults
    self.center = center
    smartQuotes = defaults.object(forKey: Self.smartQuotesKey) as? Bool ?? false
  }

  public func setSmartQuotes(_ enabled: Bool) {
    guard enabled != smartQuotes else { return }
    smartQuotes = enabled
    defaults.set(enabled, forKey: Self.smartQuotesKey)
    center.post(name: .evimEditingPreferencesDidChange, object: self)
  }
}
