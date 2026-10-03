import AppKit

/// Shared native prompting for the menu and Settings theme catalogue.
@MainActor
final class EVThemeActions {
  let store: EVThemeStore
  var requestName: (NSWindow?) -> String? = { _ in
    let prompt = EVThemeNamePrompt()
    return withExtendedLifetime(prompt) { prompt.run() }
  }

  init(store: EVThemeStore) { self.store = store }

  func create(window: NSWindow?) {
    guard let name = requestName(window) else { return }
    do { try store.createTheme(named: name) }
    catch { NSApplication.shared.presentError(error) }
  }

  func select(_ theme: EVThemeChoice?) {
    do { try store.selectTheme(named: theme?.name, fileName: theme?.fileName) }
    catch { NSApplication.shared.presentError(error) }
  }

  func revertOrDelete() {
    do {
      if store.currentThemeIsBundled { try store.revertCurrentTheme() }
      else { try store.deleteCurrentTheme() }
    }
    catch { NSApplication.shared.presentError(error) }
  }
}

@MainActor
private final class EVThemeNamePrompt: NSObject, NSTextFieldDelegate {
  private let alert = NSAlert()
  private let field = NSTextField(string: "")

  func run() -> String? {
    alert.messageText = "New Theme"
    alert.informativeText = "Choose a filename of up to 32 characters. Avoid slashes, control characters, trailing dots or spaces, and reserved names."
    alert.addButton(withTitle: "Create")
    alert.addButton(withTitle: "Cancel")
    field.placeholderString = "Theme name"
    field.setAccessibilityLabel("Theme name")
    field.frame = NSRect(x: 0, y: 0, width: 300, height: 24)
    field.delegate = self
    alert.accessoryView = field
    alert.window.initialFirstResponder = field
    validate()
    guard alert.runModal() == .alertFirstButtonReturn else { return nil }
    return field.stringValue
  }

  func controlTextDidChange(_ notification: Notification) { validate() }

  private func validate() {
    do {
      try EVConfigurationStore.validateThemeName(field.stringValue)
      alert.buttons.first?.isEnabled = true
    } catch { alert.buttons.first?.isEnabled = false }
  }
}
