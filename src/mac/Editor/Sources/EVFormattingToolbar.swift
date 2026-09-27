import AppKit
import CViemCore
import ViemAppShell

extension EVEditorSurfaceController: EVFormattingToolbarProviding {
  public var formattingToolbarFormat: EVSourceFormat { backend.sourceFormat }
  public var formattingToolbarView: NSView { formattingToolbar }
  public func refreshFormattingToolbar() { formattingToolbar.refresh() }
}

/// A presentation of the editor's existing style catalogue and menu commands.
/// No selection, document formatting, or undo history is owned by these controls.
@MainActor
final class EVFormattingToolbarView: NSView, NSMenuDelegate {
  private weak var surface: EVEditorSurfaceController?
  let paragraphStyle = NSPopUpButton()
  let characterStyle = NSPopUpButton()
  private(set) var commandButtons: [EVMenuCommand: NSButton] = [:]
  let characterCode = NSButton()
  let codeBlock = NSButton()
  let foreground = EVStyleColorWell()
  let background = EVStyleColorWell()
  private let scroll = NSScrollView()
  private let row = NSStackView()
  private let characterGroup = NSStackView()
  private let colorGroup = NSStackView()
  private let blockGroup = NSStackView()
  private let indentGroup = NSStackView()
  private var catalogue: EVStyleMenuCatalogue?
  private var styleSnapshot: EVStyleSheetSnapshot?
  private var paragraphEntries: [String] = []
  private var characterEntries: [String] = []
  private var applyingColor = false
  private var colorSelection: ViemLogicalSelectionIdentityV1?
  private var trackingMenus = Set<ObjectIdentifier>()

  init(surface: EVEditorSurfaceController) {
    self.surface = surface
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 42))
    setAccessibilityLabel("Formatting toolbar")
    row.orientation = .horizontal
    row.alignment = .centerY
    row.spacing = 12
    row.edgeInsets = NSEdgeInsets(top: 0, left: 10, bottom: 0, right: 10)
    for (popup, label, role) in [(paragraphStyle, "Paragraph style", EVStyleMenuRole.paragraph),
                                (characterStyle, "Character style", EVStyleMenuRole.character)] {
      popup.controlSize = .small
      popup.font = .systemFont(ofSize: 12)
      popup.widthAnchor.constraint(equalToConstant: 148).isActive = true
      popup.setAccessibilityLabel(label)
      popup.toolTip = label
      popup.tag = Int(role.rawValue)
      popup.target = self
      popup.action = #selector(chooseStyle(_:))
      popup.refusesFirstResponder = true
      popup.menu?.autoenablesItems = false
      popup.menu?.delegate = self
      row.addArrangedSubview(popup)
    }
    for group in [characterGroup, colorGroup, blockGroup, indentGroup] {
      group.orientation = .horizontal
      group.alignment = .centerY
      group.spacing = 2
      row.addArrangedSubview(group)
    }
    for (command, title, symbol) in [
      (EVMenuCommand.bold, "Bold", "bold"), (.italic, "Italic", "italic"),
      (.underline, "Underline", "underline"), (.strikethrough, "Strikethrough", "strikethrough")
    ] { add(command, title: title, symbol: symbol, to: characterGroup) }
    configure(characterCode, title: "Code (Character)", symbol: "chevron.left.forwardslash.chevron.right", toggle: true)
    characterCode.action = #selector(toggleCharacterCode(_:))
    characterGroup.addArrangedSubview(characterCode)
    add(.superscript, title: "Superscript", symbol: "textformat.superscript", to: characterGroup)
    add(.subscriptText, title: "Subscript", symbol: "textformat.subscript", to: characterGroup)
    for (well, title, property) in [(foreground, "Text Color", EVStyleProperty.characterForeground),
                                    (background, "Background Color", EVStyleProperty.characterBackground)] {
      well.tag = Int(property.rawValue)
      well.toolTip = title
      well.setAccessibilityLabel(title)
      well.target = self
      well.action = #selector(changeColor(_:))
      well.widthAnchor.constraint(equalToConstant: 30).isActive = true
      well.heightAnchor.constraint(equalToConstant: 18).isActive = true
      let icon = NSImageView(image: NSImage(
        systemSymbolName: property == .characterForeground ? "textformat" : "highlighter",
        accessibilityDescription: title) ?? NSImage())
      icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .regular)
      icon.setAccessibilityHidden(true)
      icon.heightAnchor.constraint(equalToConstant: 12).isActive = true
      let control = NSStackView(views: [icon, well])
      control.orientation = .vertical
      control.alignment = .centerX
      control.spacing = 1
      colorGroup.addArrangedSubview(control)
    }
    colorGroup.spacing = 8
    add(.bulletedList, title: "Bulleted List", symbol: "list.bullet", to: blockGroup)
    add(.numberedList, title: "Numbered List", symbol: "list.number", to: blockGroup)
    configure(codeBlock, title: "Code Block", symbol: "chevron.left.slash.chevron.right", toggle: true)
    codeBlock.action = #selector(toggleCodeBlock(_:))
    blockGroup.addArrangedSubview(codeBlock)
    add(.increaseIndent, title: "Indent", symbol: "increase.indent", to: indentGroup, toggle: false)
    add(.decreaseIndent, title: "Unindent", symbol: "decrease.indent", to: indentGroup, toggle: false)
    scroll.drawsBackground = false
    scroll.borderType = .noBorder
    scroll.hasHorizontalScroller = true
    scroll.autohidesScrollers = true
    scroll.scrollerStyle = .overlay
    scroll.documentView = row
    scroll.frame = bounds
    scroll.autoresizingMask = [.width, .height]
    addSubview(scroll)
    refresh()
    needsLayout = true
  }

  required init?(coder: NSCoder) { nil }

  private func configure(_ button: NSButton, title: String, symbol: String, toggle: Bool) {
    button.title = title
    button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
    button.imagePosition = .imageOnly
    button.bezelStyle = .accessoryBarAction
    button.controlSize = .small
    button.setButtonType(toggle ? .pushOnPushOff : .momentaryPushIn)
    button.allowsMixedState = toggle
    button.refusesFirstResponder = true
    button.toolTip = title
    button.setAccessibilityLabel(title)
    button.widthAnchor.constraint(equalToConstant: 27).isActive = true
    button.heightAnchor.constraint(equalToConstant: 26).isActive = true
    button.target = self
  }

  private func add(_ command: EVMenuCommand, title: String, symbol: String, to group: NSStackView, toggle: Bool = true) {
    let button = NSButton()
    configure(button, title: title, symbol: symbol, toggle: toggle)
    button.tag = command.rawValue
    button.action = #selector(performCommand(_:))
    commandButtons[command] = button
    group.addArrangedSubview(button)
  }

  override func layout() {
    super.layout()
    row.frame = NSRect(x: 0, y: 0, width: row.fittingSize.width, height: bounds.height)
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    if window == nil {
      foreground.dismissColorControls()
      background.dismissColorControls()
      colorSelection = nil
    }
  }

  func refresh() {
    guard let surface else { return }
    let selected = try? surface.session?.selectedNamedStyles()
    let selection = try? surface.session?.listSelection()
    if styleSnapshot?.identity != selected?.identity || styleSnapshot == nil {
      styleSnapshot = try? surface.backend.styleSheetSnapshot()
    }
    let next = styleSnapshot.map {
      surface.styleMenuCatalogue(snapshot: $0, selectedStyles: selected, selectionAvailable: selection != nil)
    }
    let indent = selection.flatMap { try? surface.session?.listIndentCapabilities(expected: $0) } ?? 0
    if trackingMenus.isEmpty {
      catalogue = next
      refresh(paragraphStyle, role: .paragraph)
      refresh(characterStyle, role: .character)
    }
    // Rich direct properties come from the same format capability used by
    // native typography panels; Markdown has only its fixed inline vocabulary.
    let rich = surface.canInspectTypography
    for (command, button) in commandButtons {
      let presentation: EVMenuItemPresentation
      switch command {
      case .increaseIndent, .decreaseIndent:
        let flag = UInt32(command == .increaseIndent ? VIEM_LIST_CAN_INDENT : VIEM_LIST_CAN_UNINDENT)
        presentation = EVMenuItemPresentation(isEnabled: indent & flag != 0)
      case .bulletedList, .numberedList:
        presentation = EVMenuItemPresentation(isEnabled: selected != nil,
          state: (command == .bulletedList ? selected?.bulletState : selected?.numberedState) ?? .off)
      default: presentation = surface.presentation(for: command)
      }
      if button.state != presentation.state { button.state = presentation.state }
      if button.isEnabled != presentation.isEnabled { button.isEnabled = presentation.isEnabled }
      setHidden([.underline, .strikethrough, .superscript, .subscriptText].contains(command) && !rich, for: button)
    }
    refreshCode(characterCode, role: .character, id: "Code", catalogue: next)
    refreshCode(codeBlock, role: .paragraph, id: "Code Block", catalogue: next)
    setHidden(!rich, for: colorGroup)
    colorSelection = rich ? selection : nil
    if rich, let style = try? surface.session?.selectedTypography() {
      let wasApplyingColor = applyingColor
      applyingColor = true
      defer { applyingColor = wasApplyingColor }
      let textColor = style.foreground?.appKitColor ?? EVThemeStore.shared.theme.foreground.color
      let fillColor = style.background?.appKitColor ?? .clear
      if foreground.color != textColor { foreground.color = textColor }
      if background.color != fillColor { background.color = fillColor }
    }
    let canEditColor = rich && surface.canEditTypography
    for well in [foreground, background] {
      if well.isEnabled != canEditColor { well.isEnabled = canEditColor }
      well.supportsAlpha = true
    }
  }

  private func setHidden(_ hidden: Bool, for view: NSView) {
    if view.isHidden != hidden { view.isHidden = hidden; needsLayout = true }
  }

  private func refresh(_ popup: NSPopUpButton, role: EVStyleMenuRole) {
    let entries = catalogue?.entries.filter { $0.role == role && $0.actionKind == .assign } ?? []
    let selected = entries.first { $0.presentation.state == .on }
    // Keep AppKit menu objects across cursor moves. Only catalogue membership
    // changes require rebuilding; action identities always follow the snapshot.
    let keys = (selected == nil ? ["mixed"] : []) + entries.map { "style:" + $0.stableID }
    let previous = role == .paragraph ? paragraphEntries : characterEntries
    if keys != previous {
      popup.removeAllItems()
      if selected == nil { popup.addItem(withTitle: "Mixed"); popup.lastItem?.isEnabled = false }
      for entry in entries { popup.menu?.addItem(NSMenuItem(title: entry.displayName, action: nil, keyEquivalent: "")) }
      if role == .paragraph { paragraphEntries = keys } else { characterEntries = keys }
    }
    for (index, entry) in entries.enumerated() {
      guard let item = popup.item(at: index + (selected == nil ? 1 : 0)) else { continue }
      if item.title != entry.displayName { item.title = entry.displayName }
      if item.isEnabled != entry.presentation.isEnabled { item.isEnabled = entry.presentation.isEnabled }
      if item.state != entry.presentation.state { item.state = entry.presentation.state }
      if let catalogue { item.representedObject = action(for: entry, in: catalogue) }
      if entry.stableID == selected?.stableID, popup.selectedItem !== item { popup.select(item) }
    }
    if selected == nil, popup.indexOfSelectedItem != 0 { popup.selectItem(at: 0) }
    let enabled = entries.contains { $0.presentation.isEnabled }
    if popup.isEnabled != enabled { popup.isEnabled = enabled }
  }

  private func refreshCode(_ button: NSButton, role: EVStyleMenuRole, id: String, catalogue: EVStyleMenuCatalogue?) {
    let entry = catalogue?.entries.first { $0.role == role && $0.stableID == id }
    let state = entry?.presentation.state ?? .off
    if button.state != state { button.state = state }
    let targetID = button.state == .on ? (role == .character ? "" : "Paragraph") : id
    setHidden(catalogue?.entries.first {
      $0.role == role && $0.stableID == targetID
    }?.presentation.isEnabled != true, for: button)
  }

  private func action(for entry: EVStyleMenuEntry, in catalogue: EVStyleMenuCatalogue) -> EVStyleMenuAction {
    EVStyleMenuAction(kind: .assign, role: entry.role, stableID: entry.stableID,
      documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
      styleSheetRevision: catalogue.styleSheetRevision)
  }

  func menuWillOpen(_ menu: NSMenu) { trackingMenus.insert(ObjectIdentifier(menu)) }
  func menuDidClose(_ menu: NSMenu) {
    trackingMenus.remove(ObjectIdentifier(menu))
    // AppKit dispatches the chosen item's action after ending tracking.
    DispatchQueue.main.async { [weak self] in self?.refresh() }
  }

  @objc func chooseStyle(_ sender: NSPopUpButton) {
    guard let action = sender.selectedItem?.representedObject as? EVStyleMenuAction else { return }
    surface?.perform(styleMenuAction: action, sender: sender)
    finishAction()
  }

  @objc func performCommand(_ sender: NSButton) {
    guard let command = EVMenuCommand(rawValue: sender.tag), let surface,
      surface.presentation(for: command).isEnabled else { refresh(); return }
    let removeList = (command == .bulletedList || command == .numberedList)
      && surface.presentation(for: command).state == .on
    surface.perform(menuCommand: removeList ? .removeList : command, sender: sender)
    finishAction()
  }

  @objc func toggleCharacterCode(_ sender: NSButton) { toggleStyle(role: .character, id: "Code", fallback: "", sender: sender) }
  @objc func toggleCodeBlock(_ sender: NSButton) { toggleStyle(role: .paragraph, id: "Code Block", fallback: "Paragraph", sender: sender) }

  private func toggleStyle(role: EVStyleMenuRole, id: String, fallback: String, sender: NSButton) {
    guard let surface, let catalogue = surface.currentStyleMenuCatalogue() else { return }
    let active = catalogue.entries.contains { $0.role == role && $0.stableID == id && $0.presentation.state == .on }
    guard let entry = catalogue.entries.first(where: { $0.role == role && $0.stableID == (active ? fallback : id) }),
      entry.presentation.isEnabled else { refresh(); return }
    surface.perform(styleMenuAction: action(for: entry, in: catalogue), sender: sender)
    finishAction()
  }

  @objc func changeColor(_ sender: NSColorWell) {
    guard !applyingColor, let surface, surface.canEditTypography, let session = surface.session,
      let selection = colorSelection, let rgb = sender.color.usingColorSpace(.sRGB),
      let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }
    let color = EVStyleColor(red: Float(rgb.redComponent), green: Float(rgb.greenComponent),
      blue: Float(rgb.blueComponent), alpha: Float(rgb.alphaComponent))
      .normalizedForNativePicker(format: surface.backend.sourceFormat)
    if let current = try? session.selectedFormatting(), !current.mixed.contains(property),
       current[property] == .color(color) { return }
    applyingColor = true
    defer { applyingColor = false }
    surface.performInput {
      _ = try session.editDirectProperty(property, value: .color(color), expected: selection)
    }
    refresh()
  }

  private func finishAction() {
    refresh()
    if let surface { window?.makeFirstResponder(surface.editorView) }
  }
}
