import AppKit

/// Application appearance uses portable sRGB values, independently of a
/// document's source-backed style sheet.
public struct EVThemeColor: Codable, Equatable, Sendable {
  public var red: Double
  public var green: Double
  public var blue: Double
  public var alpha: Double

  public init(_ red: Double, _ green: Double, _ blue: Double, _ alpha: Double = 1) {
    self.red = red
    self.green = green
    self.blue = blue
    self.alpha = alpha
  }

  public init(_ color: NSColor) {
    let rgb = color.usingColorSpace(.sRGB) ?? .black
    self.init(rgb.redComponent, rgb.greenComponent, rgb.blueComponent, rgb.alphaComponent)
  }

  public var color: NSColor {
    NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha)
  }

  var isValid: Bool {
    [red, green, blue, alpha].allSatisfy { $0.isFinite && (0...1).contains($0) }
  }
}

public struct EVThemePadding: Codable, Equatable, Sendable {
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

public struct EVTheme: Codable, Equatable, Sendable {
  public var foreground = EVThemeColor(0.08, 0.09, 0.11)
  public var background = EVThemeColor(1, 1, 1)
  public var statusForeground = EVThemeColor(0.88, 0.90, 0.93)
  public var statusBackground = EVThemeColor(0.12, 0.13, 0.15)
  public var caret = EVThemeColor(0.06, 0.24, 0.49)
  public var selection = EVThemeColor(0.12, 0.39, 0.73, 0.28)
  public var statusFontFamily = "System"
  public var statusFontSize: Double = 11
  public var padding = EVThemePadding()

  public init() {}
  public static let paper = EVTheme()
  public static var midnight: EVTheme {
    var theme = EVTheme()
    theme.foreground = EVThemeColor(0.90, 0.93, 0.98)
    theme.background = EVThemeColor(0.035, 0.085, 0.17)
    theme.statusBackground = EVThemeColor(0.02, 0.055, 0.12)
    theme.statusForeground = EVThemeColor(0.68, 0.78, 0.91)
    theme.caret = EVThemeColor(0.76, 0.86, 1)
    theme.selection = EVThemeColor(0.39, 0.65, 1, 0.38)
    return theme
  }

  public var statusFont: NSFont {
    if statusFontFamily == "System" { return .systemFont(ofSize: statusFontSize) }
    return NSFont(name: statusFontFamily, size: statusFontSize)
      ?? NSFontManager.shared.font(
        withFamily: statusFontFamily, traits: [], weight: 5, size: statusFontSize)
      ?? .systemFont(ofSize: statusFontSize)
  }

  var isValid: Bool {
    [foreground, background, statusForeground, statusBackground, caret, selection].allSatisfy(
      \.isValid)
      && statusFontSize.isFinite && (8...32).contains(statusFontSize)
      && !statusFontFamily.isEmpty && statusFontFamily.count < 256 && padding.isValid
  }
}

extension Notification.Name {
  public static let evimThemeDidChange = Notification.Name("com.evim.theme.did-change")
}

@MainActor
public final class EVThemeStore {
  public static let shared = EVThemeStore()
  private let configuration: EVConfigurationStore
  public var lastError: String? { configuration.lastError }
  private let center: NotificationCenter
  public private(set) var theme: EVTheme
  public private(set) var generation: UInt64 = 1
  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    theme = configuration.theme
  }

  public func update(_ value: EVTheme) {
    guard value.isValid, value != theme else { return }
    do { try configuration.setTheme(value) } catch { return }
    theme = value
    generation &+= 1
    if generation == 0 { generation = 1 }
    center.post(name: .evimThemeDidChange, object: self)
  }
}
