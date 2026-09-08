import AppKit
import ViemAppShell

extension Notification.Name {
  static let viemCaretAppearanceDidChange = Notification.Name(
    "org.viem.editor.caret-appearance-did-change"
  )
}

/// Centralizes the application caret color and invalidates every editor
/// presentation when its theme or macOS appearance/accessibility settings
/// change. The generation is presentation-only; it never enters a
/// document or layout identity.
@MainActor
final class EVCaretAppearanceResolver {
  static let shared = EVCaretAppearanceResolver()

  private let notificationCenter: NotificationCenter
  private let workspaceNotificationCenter: NotificationCenter
  private var observers: [NSObjectProtocol] = []
  private(set) var generation: UInt64 = 1

  init(
    notificationCenter: NotificationCenter = .default,
    workspaceNotificationCenter: NotificationCenter = NSWorkspace.shared.notificationCenter
  ) {
    self.notificationCenter = notificationCenter
    self.workspaceNotificationCenter = workspaceNotificationCenter

    observers.append(
      notificationCenter.addObserver(
        forName: NSColor.systemColorsDidChangeNotification,
        object: nil,
        queue: .main
      ) { [weak self] _ in
        MainActor.assumeIsolated { self?.invalidate() }
      })
    observers.append(
      notificationCenter.addObserver(
        forName: .viemThemeDidChange, object: nil, queue: .main
      ) { [weak self] _ in
        MainActor.assumeIsolated { self?.invalidate() }
      })
    observers.append(
      workspaceNotificationCenter.addObserver(
        forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
        object: nil,
        queue: .main
      ) { [weak self] _ in
        MainActor.assumeIsolated { self?.invalidate() }
      })
  }

  deinit {
    for observer in observers {
      notificationCenter.removeObserver(observer)
      workspaceNotificationCenter.removeObserver(observer)
    }
  }

  func color(for view: NSView) -> NSColor {
    EVThemeStore.shared.theme.caret.color
  }

  /// Choose the monochrome glyph color with the larger WCAG contrast ratio
  /// against an opaque caret fill. Components are converted from encoded
  /// sRGB to linear light before calculating relative luminance.
  static func glyphColor(contrastingWith caretColor: NSColor) -> NSColor {
    guard let rgb = caretColor.usingColorSpace(.sRGB) else { return .white }
    let backgroundLuminance = relativeLuminance(
      sRGBRed: rgb.redComponent,
      green: rgb.greenComponent,
      blue: rgb.blueComponent
    )
    let blackContrast = contrastRatio(
      between: backgroundLuminance,
      and: 0
    )
    let whiteContrast = contrastRatio(
      between: backgroundLuminance,
      and: 1
    )
    return blackContrast >= whiteContrast ? .black : .white
  }

  static func relativeLuminance(
    sRGBRed red: CGFloat,
    green: CGFloat,
    blue: CGFloat
  ) -> CGFloat {
    0.2126 * linearizedSRGBComponent(red)
      + 0.7152 * linearizedSRGBComponent(green)
      + 0.0722 * linearizedSRGBComponent(blue)
  }

  static func contrastRatio(
    between firstLuminance: CGFloat,
    and secondLuminance: CGFloat
  ) -> CGFloat {
    let lighter = max(firstLuminance, secondLuminance)
    let darker = min(firstLuminance, secondLuminance)
    return (lighter + 0.05) / (darker + 0.05)
  }

  func noteEffectiveAppearanceChange() {
    invalidate()
  }

  private func invalidate() {
    generation &+= 1
    if generation == 0 { generation = 1 }
    notificationCenter.post(name: .viemCaretAppearanceDidChange, object: self)
  }

  private static func linearizedSRGBComponent(_ component: CGFloat) -> CGFloat {
    let bounded = min(max(component, 0), 1)
    if bounded <= 0.04045 {
      return bounded / 12.92
    }
    return CGFloat(pow(Double((bounded + 0.055) / 1.055), 2.4))
  }
}
