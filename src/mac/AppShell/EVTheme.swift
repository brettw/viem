import AppKit

/// Application appearance uses portable sRGB values, independently of a
/// document's style sheet.
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

public struct EVTheme: Codable, Equatable, Sendable {
  public var foreground = EVThemeColor(0.90, 0.93, 0.98)
  public var background = EVThemeColor(0.035, 0.085, 0.17)
  public var statusForeground = EVThemeColor(0.68, 0.78, 0.91)
  public var statusBackground = EVThemeColor(0.02, 0.055, 0.12)
  public var caret = EVThemeColor(0.76, 0.86, 1)
  public var selection = EVThemeColor(0.39, 0.65, 1, 0.38)
  public var statusFontFamily = "System"
  public var statusFontSize: Double = 11

  public init() {}
  public static let midnight = EVTheme()
  public static var paper: EVTheme {
    var theme = EVTheme()
    theme.foreground = EVThemeColor(0.08, 0.09, 0.11)
    theme.background = EVThemeColor(1, 1, 1)
    theme.statusBackground = EVThemeColor(0.12, 0.13, 0.15)
    theme.statusForeground = EVThemeColor(0.88, 0.90, 0.93)
    theme.caret = EVThemeColor(0.06, 0.24, 0.49)
    theme.selection = EVThemeColor(0.12, 0.39, 0.73, 0.28)
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
      && !statusFontFamily.isEmpty && statusFontFamily.count < 256
  }
}

extension Notification.Name {
  public static let viemThemeDidChange = Notification.Name("com.viem.theme.did-change")
}

@MainActor
public final class EVThemeStore {
  public static let shared = EVThemeStore()
  private let configuration: EVConfigurationStore
  public var lastError: String? { configuration.lastError }
  private let center: NotificationCenter
  private var observer: NSObjectProtocol?
  public private(set) var theme: EVTheme
  public private(set) var generation: UInt64 = 1
  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    theme = configuration.theme
    observer = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: configuration, queue: .main) { [weak self] _ in
      MainActor.assumeIsolated { self?.reload() }
    }
  }

  deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }

  public func ensureCurrentThemeExists() throws { try configuration.ensureCurrentThemeExists() }
  public var currentThemeName: String? { configuration.currentThemeName }
  public var availableThemeNames: [String] { configuration.availableThemeNames }
  public var availableThemes: [EVThemeChoice] { configuration.availableThemes }
  public var currentThemeFileName: String? { configuration.selectedThemeURL?.lastPathComponent }
  public func selectTheme(named name: String?, fileName: String? = nil) throws { try configuration.selectTheme(named: name, fileName: fileName) }
  public func createTheme(named name: String) throws { try configuration.createTheme(named: name) }
  public func deleteCurrentTheme() throws { try configuration.deleteCurrentTheme() }

  private func reload() {
    theme = configuration.theme
    generation &+= 1
    if generation == 0 { generation = 1 }
    center.post(name: .viemThemeDidChange, object: self)
  }

  public func update(_ value: EVTheme) {
    guard value.isValid, value != theme else { return }
    do { try configuration.setTheme(value) } catch { return }
  }
}
