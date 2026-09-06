import AppKit

/// The editor supplies the active font's actual feature catalogue. AppShell
/// retains native menu ownership without importing a shaping implementation.
@MainActor
public protocol EVOpenTypeMenuProviding: AnyObject {
  func populateOpenTypeFeatureMenu(_ menu: NSMenu)
}
