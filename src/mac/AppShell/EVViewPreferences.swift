import Foundation

public struct EVViewMargins: Codable, Equatable, Sendable {
  public var top: Double = 28
  public var left: Double = 30
  public var bottom: Double = 28
  public var right: Double = 30
  public init(top: Double = 28, left: Double = 30, bottom: Double = 28, right: Double = 30) {
    self.top = top
    self.left = left
    self.bottom = bottom
    self.right = right
  }
  var isValid: Bool {
    [top, left, bottom, right].allSatisfy { $0.isFinite && (0...1000).contains($0) }
  }
}

extension Notification.Name {
  public static let viemViewPreferencesDidChange = Notification.Name("com.viem.view-preferences.did-change")
}

/// View geometry is an application preference, independent of document styles.
@MainActor
public final class EVViewPreferences {
  public static let shared = EVViewPreferences()
  private let configuration: EVConfigurationStore
  private let center: NotificationCenter
  public var lastError: String? { configuration.lastError }
  public private(set) var margins: EVViewMargins

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    margins = configuration.viewMargins
  }

  @discardableResult
  public func setMargins(_ value: EVViewMargins) -> Bool {
    guard value.isValid else { return false }
    guard value != margins else { return true }
    do { try configuration.setViewMargins(value) } catch { return false }
    margins = value
    center.post(name: .viemViewPreferencesDidChange, object: self)
    return true
  }
}
