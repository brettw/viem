import AppKit
import CViemCore
import CoreText
import ViemAppShell
import ViemCoreTextProvider

/// Persistent native panels follow their document selection after a short idle
/// delay. Every gesture still carries the exact selection shown by the panel.
@MainActor
final class EVTypographyPanels: NSObject {
  static let shared = EVTypographyPanels()
  private weak var fontSurface: EVEditorSurfaceController?
  private weak var colorSurface: EVEditorSurfaceController?
  private var fontSelection: ViemLogicalSelectionIdentityV1?
  private var colorSelection: ViemLogicalSelectionIdentityV1?
  private var baseFont: NSFont?
  private var colorProperty: EVStyleProperty = .characterForeground
  private var isConfiguringColorPanel = false
  private var refreshTimer: Timer?
  private var closeObserver: NSObjectProtocol?
  static let refreshDelay: TimeInterval = 0.15
  private(set) var synchronizationCount = 0

  override init() {
    super.init()
    closeObserver = NotificationCenter.default.addObserver(
      forName: NSWindow.willCloseNotification, object: nil, queue: .main
    ) { [weak self] notification in
      MainActor.assumeIsolated {
        guard let self, let window = notification.object as? NSWindow else { return }
        if window === NSFontManager.shared.fontPanel(false) {
          self.fontSurface = nil; self.fontSelection = nil
        }
        if window === NSColorPanel.shared { self.stopFollowingColors() }
        if self.fontSurface?.viewIfLoaded?.window === window {
          self.fontSurface = nil; self.fontSelection = nil
        }
        if self.colorSurface?.viewIfLoaded?.window === window { self.stopFollowingColors() }
      }
    }
  }

  deinit {
    refreshTimer?.invalidate()
    if let closeObserver { NotificationCenter.default.removeObserver(closeObserver) }
  }

  func stopFollowingColors() {
    colorSurface = nil
    colorSelection = nil
  }

  func documentDidRefresh(_ surface: EVEditorSurfaceController) {
    guard surface === fontSurface || surface === colorSurface else { return }
    let selection = try? surface.session?.listSelection()
    func differs(_ old: ViemLogicalSelectionIdentityV1?) -> Bool {
      guard var old, var selection else { return true }
      guard old.kind == selection.kind, old.view_id == selection.view_id,
        old.document_id == selection.document_id, old.document_revision == selection.document_revision,
        old.text_start == selection.text_start, old.text_end == selection.text_end else { return true }
      return withUnsafeBytes(of: &old.state_identity) { lhs in
        withUnsafeBytes(of: &selection.state_identity) { rhs in !lhs.elementsEqual(rhs) }
      }
    }
    let changedFont = surface === fontSurface && differs(fontSelection)
    let changedColor = surface === colorSurface && differs(colorSelection)
    guard changedFont || changedColor else { return }
    // Do not apply an old displayed color/font while the refresh is pending.
    if changedFont { fontSelection = nil }
    if changedColor { colorSelection = nil }
    refreshTimer?.invalidate()
    refreshTimer = Timer.scheduledTimer(withTimeInterval: Self.refreshDelay, repeats: false) { [weak self] _ in
      MainActor.assumeIsolated { self?.synchronizeNow() }
    }
  }

  func synchronizeNow() {
    refreshTimer?.invalidate()
    refreshTimer = nil
    if let surface = fontSurface, NSFontManager.shared.fontPanel(false)?.isVisible == true {
      configureFont(for: surface)
    }
    if let surface = colorSurface, NSColorPanel.shared.isVisible { configureColor(for: surface) }
    synchronizationCount += 1
  }

  private func configureFont(for surface: EVEditorSurfaceController, updateManager: Bool = true) {
    guard let session = surface.session, surface.canInspectTypography,
      let style = try? session.selectedTypography(), let selection = try? session.listSelection()
    else { fontSelection = nil; return }
    let font = resolveFont(families: [style.fontFamily], size: style.size,
      cssWeight: CGFloat(style.baseWeight), slant: style.slant, features: []) as NSFont
    baseFont = font
    fontSelection = selection
    if updateManager { NSFontManager.shared.setSelectedFont(font, isMultiple: style.mixed) }
  }

  private func configureColor(for surface: EVEditorSurfaceController) {
    guard let session = surface.session, surface.canInspectTypography,
      let style = try? session.selectedTypography(), let selection = try? session.listSelection()
    else { colorSelection = nil; return }
    isConfiguringColorPanel = true
    defer { isConfiguringColorPanel = false }
    colorSelection = selection
    let panel = NSColorPanel.shared
    panel.showsAlpha = surface.backend.sourceFormat != .rtf
    panel.color = colorProperty == .characterBackground
      ? (style.background?.appKitColor ?? .clear)
      : (style.foreground?.appKitColor ?? EVThemeStore.shared.theme.foreground.color)
  }

  func showFonts(for surface: EVEditorSurfaceController) {
    guard surface.canInspectTypography else { return }
    fontSurface = surface
    let manager = NSFontManager.shared
    manager.target = self
    manager.action = #selector(changeFont(_:))
    configureFont(for: surface)
    manager.orderFrontFontPanel(nil)
    // Explicit panel commands should move keyboard focus to the controls.
    // NSFontManager may merely order its floating panel above the document.
    manager.fontPanel(true)?.makeKeyAndOrderFront(nil)
  }

  @objc private func changeFont(_ sender: NSFontManager) {
    if fontSelection == nil, let surface = fontSurface {
      // Keep the manager's current conversion gesture while rebasing its input
      // font to the caret now being edited (for example a size-only gesture).
      configureFont(for: surface, updateManager: false)
    }
    guard let surface = fontSurface, let session = surface.session,
      surface.canEditTypography, let selection = fontSelection, let baseFont
    else { return }
    let chosen = sender.convert(baseFont)
    let faceWeight = EVFontCatalog.weight(of: chosen as CTFont)
    let baseWeight = EVFontCatalog.weight(of: baseFont as CTFont)
    let italic = CTFontGetSymbolicTraits(chosen as CTFont).contains(.traitItalic)
    let baseItalic = CTFontGetSymbolicTraits(baseFont as CTFont).contains(.traitItalic)
    var values: [(EVStyleProperty, EVStyleValue)] = []
    if chosen.fontName != baseFont.fontName {
      values.append((.characterFontFamilies, .stringList([chosen.fontName])))
    }
    if chosen.pointSize != baseFont.pointSize {
      values.append((.characterSize, .float(Float(chosen.pointSize))))
    }
    if faceWeight != baseWeight {
      values.append((.characterWeight, .unsigned(UInt32(faceWeight))))
      // Base face weight and semantic Bold are independent properties.
      if let current = try? session.selectedFormatting(),
        !current.mixed.contains(.characterBold), let bold = current[.characterBold] {
        values.append((.characterBold, bold))
      }
    }
    if italic != baseItalic { values.append((.characterSlant, .fontSlant(italic ? 1 : 0))) }
    guard !values.isEmpty else { return }
    surface.performInput {
      _ = try session.setDirectCharacterProperties(values, expected: selection)
      self.baseFont = chosen
      self.fontSelection = try session.listSelection()
    }
  }

  func showColors(for surface: EVEditorSurfaceController, highlight: Bool) {
    guard surface.canInspectTypography else { return }
    EVStyleColorWell.deactivatePanelOwner()
    colorSurface = surface
    colorProperty = highlight ? .characterBackground : .characterForeground
    let panel = NSColorPanel.shared
    panel.setTarget(self)
    panel.setAction(#selector(changeColor(_:)))
    panel.isContinuous = false
    configureColor(for: surface)
    panel.makeKeyAndOrderFront(nil)
  }

  @objc private func changeColor(_ sender: NSColorPanel) {
    guard !isConfiguringColorPanel else { return }
    let chosenColor = sender.color
    if colorSelection == nil { synchronizeNow() }
    guard
      let surface = colorSurface, surface.canEditTypography,
      let session = surface.session, let selection = colorSelection,
      let color = chosenColor.usingColorSpace(.sRGB)
    else { return }
    let property = colorProperty
    surface.performInput {
      let nativeColor = EVStyleColor(
        red: Float(color.redComponent), green: Float(color.greenComponent),
        blue: Float(color.blueComponent), alpha: Float(color.alphaComponent)
      )
      .normalizedForNativePicker(format: surface.backend.sourceFormat)
      _ = try session.editDirectProperty(property, value: .color(nativeColor), expected: selection)
      self.colorSelection = try session.listSelection()
    }
  }
}

extension EVStyleColor {
  /// A native picker authors values in the document's color precision. RTF
  /// stores integer RGB bytes; the portable translator still verifies exact
  /// representation of the value submitted by this control.
  func normalizedForNativePicker(format: EVSourceFormat) -> EVStyleColor {
    guard format == .rtf else { return self }
    return EVStyleColor(
      red: (red * 255).rounded() / 255,
      green: (green * 255).rounded() / 255, blue: (blue * 255).rounded() / 255,
      alpha: alpha == 0 ? 0 : 1)
  }
}

extension EVEditorSurfaceController {
  var canInspectTypography: Bool {
    backend.sourceFormat == .rtf && session != nil
  }

  var canEditTypography: Bool {
    canInspectTypography && !isVisualBlockMode
      && (try? session?.listSelection()).map { $0.text_start < $0.text_end || viewPresentation.mode == UInt32(VIEM_MODE_INSERT) || viewPresentation.mode == UInt32(VIEM_MODE_REPLACE) } == true
  }
}
