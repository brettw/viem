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
  public static var editVisibleWhitespaceStyle: (@MainActor (EVConfigurationStore) -> Void)?
  public static var validateWhitespacePresentation: (@MainActor (EVWhitespacePresentationOptions) -> String?)?
  public let configuration: EVConfigurationStore
  public var lastError: String? { validationError ?? configuration.lastError }
  private var validationError: String?
  private var configurationObserver: NSObjectProtocol?
  private let center: NotificationCenter
  public private(set) var smartQuotes: Bool
  /// Application default `textwidth` in columns for `gq`/`gw` reflow.
  public private(set) var textWidth: UInt32
  public private(set) var indentation: EVIndentationOptions
  public private(set) var whitespacePresentation: EVWhitespacePresentationOptions

  public init(configuration: EVConfigurationStore? = nil, center: NotificationCenter = .default) {
    let configuration = configuration ?? .shared
    self.configuration = configuration
    self.center = center
    smartQuotes = configuration.smartQuotes
    textWidth = configuration.textWidth
    indentation = configuration.indentation
    whitespacePresentation = configuration.whitespacePresentation
    configurationObserver = NotificationCenter.default.addObserver(
      forName: .viemConfigurationDidChange, object: nil, queue: .main
    ) { [weak self] notification in
      MainActor.assumeIsolated {
        guard let self, let changed = notification.object as? EVConfigurationStore,
              changed.directory.standardizedFileURL == self.configuration.directory.standardizedFileURL else { return }
        self.reloadFromConfiguration()
      }
    }
  }

  deinit {
    if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
  }

  public func reloadFromConfiguration() {
    do { try configuration.reloadFromDisk() } catch { return }
    let changed = smartQuotes != configuration.smartQuotes || textWidth != configuration.textWidth
      || indentation != configuration.indentation || whitespacePresentation != configuration.whitespacePresentation
    smartQuotes = configuration.smartQuotes
    textWidth = configuration.textWidth
    indentation = configuration.indentation
    whitespacePresentation = configuration.whitespacePresentation
    validationError = nil
    if changed { center.post(name: .viemEditingPreferencesDidChange, object: self) }
  }

  @discardableResult
  public func setIndentation(_ options: EVIndentationOptions) -> Bool {
    guard options != indentation else { return true }
    do { try configuration.setIndentation(options) } catch { validationError = error.localizedDescription; return false }
    validationError = nil
    reloadFromConfiguration()
    return true
  }

  @discardableResult
  public func setWhitespacePresentation(_ options: EVWhitespacePresentationOptions) -> Bool {
    guard options != whitespacePresentation else { return true }
    do { try configuration.setWhitespacePresentation(options) } catch { validationError = error.localizedDescription; return false }
    validationError = nil
    reloadFromConfiguration()
    return true
  }

  public func openVisibleWhitespaceStyle() { Self.editVisibleWhitespaceStyle?(configuration) }

  /// Rejects zero without changing the prior value. Returns whether the
  /// requested width is now the effective default.
  @discardableResult
  public func setTextWidth(_ width: UInt32) -> Bool {
    guard width != textWidth else { return true }
    guard width > 0 else { return false }
    do { try configuration.setTextWidth(width) } catch { return false }
    reloadFromConfiguration()
    return true
  }

  public func setSmartQuotes(_ enabled: Bool) {
    guard enabled != smartQuotes else { return }
    do { try configuration.setSmartQuotes(enabled) } catch { return }
    reloadFromConfiguration()
  }
}
