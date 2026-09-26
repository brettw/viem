import AppKit

/// The shell owns window chrome; the editor supplies controls backed by the
/// same portable selections, capabilities, and intentions as the menus.
@MainActor
public protocol EVFormattingToolbarProviding: AnyObject {
  var formattingToolbarFormat: EVSourceFormat { get }
  var formattingToolbarView: NSView { get }
  func refreshFormattingToolbar()
}

@MainActor
final class EVFormattingToolbarChrome: NSObject {
  private weak var window: NSWindow?
  private weak var surface: (any EVEditorSurface)?
  private let configuration: EVConfigurationStore
  private let toggleAccessory = NSTitlebarAccessoryViewController()
  private let toolbarAccessory = NSTitlebarAccessoryViewController()
  let toggleButton = NSButton()
  private var installedView: NSView?
  private var updating = false
  private var configurationObserver: NSObjectProtocol?

  init(window: NSWindow, configuration: EVConfigurationStore) {
    self.window = window
    self.configuration = configuration
    super.init()
    toggleAccessory.layoutAttribute = .right
    toggleAccessory.view = NSView(frame: NSRect(x: 0, y: 0, width: 42, height: 28))
    toggleButton.frame = NSRect(x: 3, y: 2, width: 30, height: 24)
    toggleButton.bezelStyle = .accessoryBarAction
    toggleButton.setButtonType(.pushOnPushOff)
    // Keep the native neutral bezel instead of tinting the selected state.
    // Visibility is indicated by the bezel itself; off is transparent.
    (toggleButton.cell as? NSButtonCell)?.showsStateBy = []
    toggleButton.image = Self.toggleImage()
    toggleButton.imagePosition = .imageOnly
    toggleButton.title = ""
    toggleButton.target = self
    toggleButton.action = #selector(toggleToolbar(_:))
    toggleButton.refusesFirstResponder = true
    toggleButton.setAccessibilityLabel("Formatting toolbar")
    toggleAccessory.view.addSubview(toggleButton)
    toolbarAccessory.layoutAttribute = .bottom
    toolbarAccessory.view = NSView(frame: NSRect(x: 0, y: 0, width: window.frame.width, height: 42))
    configurationObserver = NotificationCenter.default.addObserver(
      forName: .viemConfigurationDidChange, object: configuration, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated {
        guard let self, let surface = self.surface else { return }
        self.synchronize(surface: surface)
      }
    }
  }

  deinit {
    if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
  }

  func synchronize(surface: any EVEditorSurface) {
    guard !updating, let window else { return }
    updating = true
    defer { updating = false }
    self.surface = surface
    guard let provider = surface as? any EVFormattingToolbarProviding,
      provider.formattingToolbarFormat != .plainText, provider.formattingToolbarFormat != .code
    else {
      remove(toolbarAccessory, from: window)
      remove(toggleAccessory, from: window)
      installedView?.removeFromSuperview()
      installedView = nil
      return
    }
    install(toggleAccessory, in: window)
    let visible = configuration.showFormattingToolbar(for: provider.formattingToolbarFormat)
    toggleButton.state = visible ? .on : .off
    toggleButton.isBordered = visible
    toggleButton.toolTip = visible ? "Hide Formatting Toolbar" : "Show Formatting Toolbar"
    if visible {
      let controls = provider.formattingToolbarView
      if installedView !== controls {
        installedView?.removeFromSuperview()
        controls.frame = toolbarAccessory.view.bounds
        controls.autoresizingMask = [.width, .height]
        toolbarAccessory.view.addSubview(controls)
        installedView = controls
      }
      install(toolbarAccessory, in: window)
      provider.refreshFormattingToolbar()
    } else {
      remove(toolbarAccessory, from: window)
      installedView?.removeFromSuperview()
      installedView = nil
    }
  }

  @objc func toggleToolbar(_ sender: Any?) {
    guard let surface, let provider = surface as? any EVFormattingToolbarProviding else { return }
    do {
      let format = provider.formattingToolbarFormat
      try configuration.setShowFormattingToolbar(!configuration.showFormattingToolbar(for: format), for: format)
    } catch { surface.showDocumentMessage(error.localizedDescription) }
    synchronize(surface: surface)
  }

  private func install(_ accessory: NSTitlebarAccessoryViewController, in window: NSWindow) {
    if !window.titlebarAccessoryViewControllers.contains(where: { $0 === accessory }) {
      window.addTitlebarAccessoryViewController(accessory)
    }
  }

  private func remove(_ accessory: NSTitlebarAccessoryViewController, from window: NSWindow) {
    if let index = window.titlebarAccessoryViewControllers.firstIndex(where: { $0 === accessory }) {
      window.removeTitlebarAccessoryViewController(at: index)
    }
  }

  private static func toggleImage() -> NSImage {
    let image = NSImage(size: NSSize(width: 24, height: 18), flipped: false) { _ in
      NSColor.black.setStroke()
      let outline = NSBezierPath(roundedRect: NSRect(x: 1.5, y: 3.5, width: 21, height: 11), xRadius: 1.5, yRadius: 1.5)
      outline.lineWidth = 1
      outline.stroke()
      for x: CGFloat in [4, 10, 16] {
        let square = NSBezierPath(rect: NSRect(x: x, y: 7, width: 4, height: 4))
        square.lineWidth = 1
        square.stroke()
      }
      return true
    }
    image.isTemplate = true
    return image
  }
}
