import AppKit
import CViemCore
import CoreText
import ViemAppShell
import ViemCoreTextProvider

/// Native panels retain the exact selection that opened them. An intervening
/// cursor move or document edit cannot redirect a panel gesture to other text.
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

  func showFonts(for surface: EVEditorSurfaceController) {
    guard let session = surface.session, let style = try? session.selectedTypography(),
      let selection = try? session.listSelection(), surface.canEditTypography
    else { return }
    fontSurface = surface
    fontSelection = selection
    let font =
      resolveFont(
        families: [style.fontFamily], size: style.size,
        cssWeight: CGFloat(style.baseWeight), slant: style.slant, features: []) as NSFont
    baseFont = font
    let manager = NSFontManager.shared
    manager.target = self
    manager.action = #selector(changeFont(_:))
    manager.setSelectedFont(font, isMultiple: style.mixed)
    manager.orderFrontFontPanel(nil)
    // Explicit panel commands should move keyboard focus to the controls.
    // NSFontManager may merely order its floating panel above the document.
    manager.fontPanel(true)?.makeKeyAndOrderFront(nil)
  }

  @objc private func changeFont(_ sender: NSFontManager) {
    guard let surface = fontSurface, let session = surface.session,
      let selection = fontSelection, let baseFont
    else { return }
    let chosen = sender.convert(baseFont)
    let faceWeight = EVFontCatalog.weight(of: chosen as CTFont)
    let italic = CTFontGetSymbolicTraits(chosen as CTFont).contains(.traitItalic)
    surface.performInput {
      _ = try session.setDirectCharacterProperties(
        [
          (.characterFontFamilies, .stringList([chosen.fontName])),
          (.characterWeight, .unsigned(UInt32(faceWeight))),
          (.characterSize, .float(Float(chosen.pointSize))),
          (.characterSlant, .fontSlant(italic ? 1 : 0)),
        ], expected: selection)
      self.baseFont = chosen
      self.fontSelection = try session.listSelection()
    }
  }

  func showColors(for surface: EVEditorSurfaceController, highlight: Bool) {
    guard let session = surface.session, let style = try? session.selectedTypography(),
      let selection = try? session.listSelection(), surface.canEditTypography
    else { return }
    colorSurface = surface
    colorSelection = selection
    colorProperty = highlight ? .characterBackground : .characterForeground
    isConfiguringColorPanel = true
    defer { isConfiguringColorPanel = false }
    let panel = NSColorPanel.shared
    panel.setTarget(self)
    panel.setAction(#selector(changeColor(_:)))
    panel.isContinuous = false
    panel.showsAlpha = surface.backend.sourceFormat != .rtf
    panel.color =
      highlight
      ? .clear : (style.foreground?.appKitColor ?? EVThemeStore.shared.theme.foreground.color)
    panel.makeKeyAndOrderFront(nil)
  }

  @objc private func changeColor(_ sender: NSColorPanel) {
    guard !isConfiguringColorPanel,
      let surface = colorSurface, let session = surface.session, let selection = colorSelection,
      let color = sender.color.usingColorSpace(.deviceRGB)
    else { return }
    let property = colorProperty
    surface.performInput {
      let nativeColor = EVStyleColor(
        red: Float(color.redComponent), green: Float(color.greenComponent),
        blue: Float(color.blueComponent), alpha: Float(color.alphaComponent)
      )
      .normalizedForNativePicker(format: surface.backend.sourceFormat)
      let value: EVStyleValue? =
        property == .characterBackground && color.alphaComponent == 0
        ? nil
        : .color(nativeColor)
      _ = try session.editDirectProperty(property, value: value, expected: selection)
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
  var canEditTypography: Bool {
    [.html, .htmlSource, .rtf].contains(backend.sourceFormat)
      && (try? session?.listSelection()).map { $0.text_start < $0.text_end || viewPresentation.mode == UInt32(VIEM_MODE_INSERT) || viewPresentation.mode == UInt32(VIEM_MODE_REPLACE) } == true
  }

  func changeFontSize(increasing: Bool) {
    guard let session else { return }
    performInput {
      let style = try session.selectedTypography()
      _ = try session.editDirectProperty(
        .characterSize,
        value: .float(Float(max(1, style.size + (increasing ? 1 : -1)))),
        expected: session.listSelection())
    }
  }
}
