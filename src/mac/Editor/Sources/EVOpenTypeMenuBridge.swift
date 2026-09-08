import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

extension EVEditorSurfaceController: EVOpenTypeMenuProviding {
  public func populateOpenTypeFeatureMenu(_ menu: NSMenu) {
    menu.removeAllItems()
    menu.autoenablesItems = false
    guard let session, let typography = try? session.selectedTypography(),
      let selection = try? session.listSelection()
    else { return }
    let features = EVFontCatalog.features(for: typography.fontFamily)
    for feature in features {
      let enabled =
        typography.features.first { $0.tag == feature.tag }?.setting ?? feature.defaultValue
      let action = EVOpenTypeMenuAction(
        surface: self, selection: selection,
        current: typography.features, tag: feature.tag, enabled: enabled == 0)
      let item = NSMenuItem(
        title: feature.label, action: #selector(EVOpenTypeMenuAction.applyFeature(_:)),
        keyEquivalent: "")
      item.state = enabled == 0 ? .off : .on
      item.target = action
      item.representedObject = action
      item.isEnabled = canEditTypography
      menu.addItem(item)
    }
    if features.isEmpty {
      let item = NSMenuItem(
        title: "No optional features in this font", action: nil, keyEquivalent: "")
      item.isEnabled = false
      menu.addItem(item)
    } else {
      menu.addItem(.separator())
      let action = EVOpenTypeMenuAction(
        surface: self, selection: selection,
        current: typography.features, tag: nil, enabled: false)
      let item = NSMenuItem(
        title: "Use Font Defaults", action: #selector(EVOpenTypeMenuAction.applyFeature(_:)),
        keyEquivalent: "")
      item.target = action
      item.representedObject = action
      item.isEnabled = canEditTypography
      menu.addItem(item)
    }
  }
}

@MainActor
private final class EVOpenTypeMenuAction: NSObject {
  weak var surface: EVEditorSurfaceController?
  let selection: ViemLogicalSelectionIdentityV1
  let current: [EVOpenTypeFeature]
  let tag: String?
  let enabled: Bool
  init(
    surface: EVEditorSurfaceController, selection: ViemLogicalSelectionIdentityV1,
    current: [EVOpenTypeFeature], tag: String?, enabled: Bool
  ) {
    self.surface = surface
    self.selection = selection
    self.current = current
    self.tag = tag
    self.enabled = enabled
  }
  @objc func applyFeature(_ sender: Any?) {
    guard let surface, let session = surface.session else { return }
    var features = current
    if let tag {
      features.removeAll { $0.tag == tag }
      features.append(EVOpenTypeFeature(tag: tag, setting: enabled ? 1 : 0))
    }
    surface.performInput {
      _ = try session.editDirectProperty(
        .characterOpenTypeFeatures,
        value: self.tag == nil ? nil : .openTypeFeatures(features.sorted { $0.tag < $1.tag }),
        expected: self.selection)
    }
  }
}
