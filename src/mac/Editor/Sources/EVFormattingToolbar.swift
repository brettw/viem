import AppKit
import CViemCore
import ViemAppShell

extension EVEditorSurfaceController: EVFormattingToolbarProviding {
  public var formattingToolbarFormat: EVSourceFormat { backend.sourceFormat }
  public var isLiteralMarkdown: Bool { backend.sourceFormat == .code && isMarkdownDocument }
  public var formattingToolbarView: NSView { formattingToolbar }
  public func refreshFormattingToolbar() { formattingToolbar.refresh() }
  public func showMarkdownPreview() {
    setMarkdownPreview(true)
    if isViewLoaded { editorView.window?.makeFirstResponder(editorView) }
  }
}

/// A presentation of the editor's existing style catalogue and document actions.
/// No selection, document formatting, or undo history is owned by these controls.
@MainActor
final class EVFormattingToolbarView: NSView, NSMenuDelegate {
  private weak var surface: EVEditorSurfaceController?
  let paragraphStyle = NSPopUpButton()
  private(set) var commandButtons: [EVMenuCommand: NSButton] = [:]
  let characterCode = NSButton()
  let codeBlock = NSButton()
  let blockQuote = NSButton()
  /// Pops up the GitHub alert kinds beside the quote toggle.
  let blockQuoteAlerts = NSButton()
  let formattedView = NSButton()
  let insertTable = EVInsertTableButton()
  let insertLink = NSButton()
  let insertReference = NSButton()
  let insertImage = NSButton()
  private(set) var tablePicker: EVTablePickerController?
  private(set) var menuTablePicker: EVTablePickerController?
  private var pickerSelection: ViemLogicalSelectionIdentityV1?
  private let controlsViewport = NSView()
  private let row = NSStackView()
  private let characterGroup = NSStackView()
  private let scriptGroup = NSStackView()
  private let codeLinkGroup = NSStackView()
  private let blockGroup = NSStackView()
  private let indentGroup = NSStackView()
  private var catalogue: EVStyleMenuCatalogue?
  private var styleSnapshot: EVStyleSheetSnapshot?
  private var paragraphEntries: [String] = []
  private var trackingMenus = Set<ObjectIdentifier>()

  init(surface: EVEditorSurfaceController) {
    self.surface = surface
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 42))
    setAccessibilityLabel("Formatting toolbar")
    row.orientation = .horizontal
    row.alignment = .centerY
    row.spacing = 8
    row.edgeInsets = NSEdgeInsets(top: 0, left: 10, bottom: 0, right: 10)
    for (popup, label, role) in [(paragraphStyle, "Paragraph style", EVStyleMenuRole.paragraph)] {
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
    }
    for group in [characterGroup, scriptGroup, codeLinkGroup, blockGroup, indentGroup] {
      group.orientation = .horizontal
      group.alignment = .centerY
      group.spacing = 2
    }
    row.addArrangedSubview(characterGroup)
    row.addArrangedSubview(scriptGroup)
    row.addArrangedSubview(codeLinkGroup)
    row.addArrangedSubview(blockGroup)
    for (command, title, symbol) in [
      (EVMenuCommand.bold, "Bold", "bold"), (.italic, "Italic", "italic"), (.underline, "Underline", "underline"),
      (.strikethrough, "Strikethrough", "strikethrough")
    ] { add(command, title: title, symbol: symbol, to: characterGroup) }
    add(.superscript, title: "Superscript", to: scriptGroup, image: EVStyleIcons.scriptImage(raised: true))
    add(.subscript, title: "Subscript", to: scriptGroup, image: EVStyleIcons.scriptImage(raised: false))
    configure(characterCode, title: "Code (Character)", symbol: "chevron.left.forwardslash.chevron.right", toggle: true)
    characterCode.toolTip = shortcutToolTip("Code (Character)", command: .characterCode)
    characterCode.action = #selector(toggleCharacterCode(_:))
    codeLinkGroup.addArrangedSubview(characterCode)
    configure(insertLink, title: "Link", symbol: "link", toggle: true)
    insertLink.toolTip = shortcutToolTip("Link", command: .insertLink)
    insertLink.action = #selector(openLinkEditor(_:))
    codeLinkGroup.addArrangedSubview(insertLink)
    configure(insertReference, title: "Reference", toggle: true)
    insertReference.title = "[d]"
    insertReference.imagePosition = .noImage
    insertReference.font = .monospacedSystemFont(ofSize: 11, weight: .medium)
    insertReference.action = #selector(openReferenceEditor(_:))
    codeLinkGroup.addArrangedSubview(insertReference)
    add(.bulletedList, title: "Bulleted List", symbol: "list.bullet", to: blockGroup)
    add(.numberedList, title: "Numbered List", symbol: "list.number", to: blockGroup)
    add(.taskList, title: "Task List", symbol: "checklist", to: blockGroup)
    let listSeparator = NSBox()
    listSeparator.boxType = .separator
    listSeparator.widthAnchor.constraint(equalToConstant: 1).isActive = true
    listSeparator.heightAnchor.constraint(equalToConstant: 18).isActive = true
    if let taskList = commandButtons[.taskList] { blockGroup.setCustomSpacing(8, after: taskList) }
    blockGroup.addArrangedSubview(listSeparator)
    blockGroup.setCustomSpacing(8, after: listSeparator)
    configure(blockQuote, title: "Block Quote", toggle: true)
    blockQuote.image = Self.blockQuoteImage()
    blockQuote.action = #selector(toggleBlockQuote(_:))
    blockGroup.addArrangedSubview(blockQuote)
    configure(blockQuoteAlerts, title: "Alert Type", symbol: "chevron.down", toggle: false)
    blockQuoteAlerts.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 8, weight: .semibold)
    blockQuoteAlerts.constraints.first { $0.firstAttribute == .width }?.constant = 13
    blockQuoteAlerts.action = #selector(showAlertMenu(_:))
    blockGroup.addArrangedSubview(blockQuoteAlerts)
    blockGroup.setCustomSpacing(0, after: blockQuote)
    configure(codeBlock, title: "Code Block", symbol: "curlybraces", toggle: true)
    codeBlock.action = #selector(toggleCodeBlock(_:))
    blockGroup.addArrangedSubview(codeBlock)
    configure(insertImage, title: "Insert Image", symbol: "photo", toggle: true)
    insertImage.action = #selector(openImageEditor(_:))
    blockGroup.addArrangedSubview(insertImage)
    row.addArrangedSubview(insertTable)
    row.addArrangedSubview(indentGroup)
    add(.increaseIndent, title: "Indent", symbol: "increase.indent", to: indentGroup, toggle: false)
    add(.decreaseIndent, title: "Unindent", symbol: "decrease.indent", to: indentGroup, toggle: false)
    row.addArrangedSubview(paragraphStyle)
    configure(insertTable, title: "Insert Table", toggle: false)
    insertTable.image = Self.tableImage()
    insertTable.openPicker = { [weak self] event in self?.openTablePicker(event: event) }
    controlsViewport.clipsToBounds = true
    controlsViewport.addSubview(row)
    addSubview(controlsViewport)
    configure(formattedView, title: "Markdown Preview", toggle: true)
    formattedView.image = EVFormattingToolbarIcons.markdownPreview()
    formattedView.allowsMixedState = false
    formattedView.toolTip = "Markdown Preview"
    formattedView.action = #selector(toggleMarkdownPreview(_:))
    addSubview(formattedView)
    refresh()
    needsLayout = true
  }

  required init?(coder: NSCoder) { nil }

  private func configure(_ button: NSButton, title: String, symbol: String? = nil, toggle: Bool) {
    button.title = title
    button.image = symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: title) }
    button.imagePosition = .imageOnly
    button.bezelStyle = .accessoryBarAction
    button.controlSize = .small
    button.setButtonType(toggle ? .pushOnPushOff : .momentaryPushIn)
    button.allowsMixedState = toggle
    button.refusesFirstResponder = true
    button.toolTip = title
    button.setAccessibilityLabel(title)
    button.widthAnchor.constraint(equalToConstant: button === insertReference ? 35 : 27).isActive = true
    button.heightAnchor.constraint(equalToConstant: 26).isActive = true
    button.target = self
  }

  private func add(_ command: EVMenuCommand, title: String, symbol: String? = nil, to group: NSStackView, image: NSImage? = nil, toggle: Bool = true) {
    let button = NSButton()
    configure(button, title: title, symbol: symbol, toggle: toggle)
    button.toolTip = shortcutToolTip(title, command: command)
    if let image { button.image = image }
    button.tag = command.rawValue
    button.action = #selector(performCommand(_:))
    commandButtons[command] = button
    group.addArrangedSubview(button)
  }

  private func shortcutToolTip(_ title: String, command: EVMenuCommand) -> String {
    command.formattingShortcut.map { "\(title) (\($0.label))" } ?? title
  }

  override func layout() {
    super.layout()
    formattedView.frame = NSRect(x: max(0, bounds.width - 37), y: (bounds.height - 26) / 2,
                                 width: 27, height: 26)
    // Keep natural-width controls fixed and clipped before the view toggle.
    controlsViewport.frame = NSRect(x: 0, y: 0, width: max(0, formattedView.frame.minX - 12), height: bounds.height)
    row.frame = NSRect(x: 0, y: 0, width: row.fittingSize.width, height: bounds.height)
  }

  private static func tableImage() -> NSImage {
    let image = NSImage(size: NSSize(width: 18, height: 15), flipped: false) { _ in
      NSColor.black.setStroke()
      let path = NSBezierPath()
      for x in [CGFloat(1), 9, 17] { path.move(to: NSPoint(x: x, y: 1.5)); path.line(to: NSPoint(x: x, y: 13.5)) }
      for y in [CGFloat(1.5), 5.5, 9.5, 13.5] { path.move(to: NSPoint(x: 1, y: y)); path.line(to: NSPoint(x: 17, y: y)) }
      path.lineWidth = 1; path.stroke(); return true
    }
    image.isTemplate = true; return image
  }

  private static func blockQuoteImage() -> NSImage {
    let image = NSImage(size: NSSize(width: 20, height: 14), flipped: false) { _ in
      NSColor.black.setStroke()
      NSColor.black.setFill()
      let outline = NSBezierPath(rect: NSRect(x: 1.5, y: 1.5, width: 17, height: 11))
      outline.lineWidth = 1
      outline.stroke()
      NSRect(x: 1, y: 1, width: 4, height: 12).fill()
      return true
    }
    image.isTemplate = true
    return image
  }

  func openTablePicker(event: NSEvent? = nil) {
    guard let surface, let session = surface.session,
          let context = try? session.tableContext(), context.flags & 1 != 0 else { return }
    let picker = tablePicker ?? EVTablePickerController()
    tablePicker = picker
    pickerSelection = context.selection
    picker.didClose = { [weak self] in self?.pickerSelection = nil }
    picker.open(from: insertTable, editor: surface.editorView, event: event) { [weak self, weak surface, weak session] columns, rows in
      guard let self, let surface, let session, surface.session === session else { return }
      surface.performInput { _ = try session.insertTable(columns: columns, bodyRows: rows, expected: context.selection) }
      self.finishAction()
    }
  }

  func makeTableMenuView(in menu: NSMenu) -> NSView? {
    guard let surface, let session = surface.session,
          let context = try? session.tableContext(), context.flags & 1 != 0 else { return nil }
    menuTablePicker?.close()
    let picker = EVTablePickerController()
    menuTablePicker = picker
    return picker.menuView(in: menu, editor: surface.editorView) { [weak self, weak surface, weak session] columns, rows in
      guard let self, let surface, let session, surface.session === session else { return }
      surface.performInput { _ = try session.insertTable(columns: columns, bodyRows: rows, expected: context.selection) }
      self.finishAction()
    }
  }

  func refresh() {
    guard let surface else { return }
    let markdown = surface.backend.sourceFormat == .markdown
    formattedView.state = markdown ? .on : .off
    formattedView.isEnabled = surface.isMarkdownDocument
    // Only Preview has formatting controls; with Preview off the title bar
    // offers Preview in place of this detached toolbar.
    controlsViewport.isHidden = !markdown
    setHidden(!markdown, for: insertTable)
    setHidden(!markdown, for: insertLink)
    let link = surface.linkPopover.toolbarPresentation
    insertLink.state = link.state
    insertLink.isEnabled = markdown && link.isEnabled
    setHidden(!markdown, for: insertReference)
    let reference = surface.linkPopover.referenceToolbarPresentation
    insertReference.state = reference.state
    insertReference.isEnabled = markdown && reference.isEnabled
    setHidden(!markdown, for: insertImage)
    let image = surface.imagePopover.toolbarPresentation
    insertImage.state = image.state
    insertImage.isEnabled = markdown && image.isEnabled
    let tableContext = try? surface.session?.tableContext()
    insertTable.isEnabled = markdown && (tableContext?.flags ?? 0) & 1 != 0
    if let expected = pickerSelection, let current = tableContext?.selection,
       !expected.isSameSelection(as: current) {
      tablePicker?.close()
    }
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
    }
    for (command, button) in commandButtons {
      let presentation: EVMenuItemPresentation
      switch command {
      case .bulletedList, .numberedList, .taskList:
        presentation = surface.listStylePresentation(command, selected: selected)
      case .increaseIndent, .decreaseIndent:
        let flag = UInt32(command == .increaseIndent ? VIEM_LIST_CAN_INDENT : VIEM_LIST_CAN_UNINDENT)
        presentation = EVMenuItemPresentation(isEnabled: indent & flag != 0)
      default: presentation = surface.presentation(for: command)
      }
      if button.state != presentation.state { button.state = presentation.state }
      if button.isEnabled != presentation.isEnabled { button.isEnabled = presentation.isEnabled }
    }
    refreshCode(characterCode, role: .character, id: "Code", catalogue: next)
    refreshCode(codeBlock, role: .paragraph, id: "Code Block", catalogue: next)
    blockQuote.state = selected?.quoteState ?? .off
    blockQuote.isEnabled = selected?.hasCodeBlock == false && next?.entries.contains { $0.role == .paragraph && $0.stableID == "Block quote" && $0.presentation.isEnabled } == true
    blockQuoteAlerts.isEnabled = EVMenuCommand.alertCommands.contains { command in
      command.alertKind.map { surface.alertPresentation($0).isEnabled } ?? false
    }
    if selected?.hasTable == true {
      paragraphStyle.isEnabled = false
      for action in [EVMenuCommand.bulletedList, .numberedList, .taskList, .increaseIndent, .decreaseIndent] { commandButtons[action]?.isEnabled = false }
      setHidden(true, for: codeBlock)
      blockQuote.isEnabled = false
      blockQuoteAlerts.isEnabled = false
    }
  }

  private func setHidden(_ hidden: Bool, for view: NSView) {
    if view.isHidden != hidden { view.isHidden = hidden; needsLayout = true }
  }

  private func refresh(_ popup: NSPopUpButton, role: EVStyleMenuRole) {
    let definitions = Dictionary(uniqueKeysWithValues: (styleSnapshot?.definitions ?? []).map { ($0.key, $0) })
    let entries = catalogue?.entries.filter {
      guard $0.role == role && $0.actionKind == .assign else { return false }
      guard role == .paragraph, surface?.backend.sourceFormat == .markdown else { return true }
      guard let definition = definitions[EVStyleKey(namespace: .block, id: EVStyleID(rawValue: $0.stableID))] else { return false }
      return definition.capabilities.contains(.assign)
        || $0.presentation.state == .on && definition.flags.contains(.internalList)
    } ?? []
    let selected = entries.first { $0.presentation.state == .on }
    // Keep AppKit menu objects across cursor moves. Only catalogue membership
    // changes require rebuilding; action identities always follow the snapshot.
    let keys = (selected == nil ? ["mixed"] : []) + entries.map { "style:" + $0.stableID }
    let previous = paragraphEntries
    if keys != previous {
      popup.removeAllItems()
      if selected == nil {
        popup.addItem(withTitle: "Mixed")
        popup.lastItem?.isEnabled = false
        popup.lastItem?.image = EVStyleIcons.typeImage(role == .paragraph ? .paragraph : .character)
      }
      for entry in entries { popup.menu?.addItem(NSMenuItem(title: entry.displayName, action: nil, keyEquivalent: "")) }
      paragraphEntries = keys
    }
    for (index, entry) in entries.enumerated() {
      guard let item = popup.item(at: index + (selected == nil ? 1 : 0)) else { continue }
      if item.title != entry.displayName { item.title = entry.displayName }
      let key = EVStyleKey(namespace: role == .paragraph ? .block : .character, id: EVStyleID(rawValue: entry.stableID))
      let typeImage = definitions[key].map { EVStyleIcons.typeImage(for: $0) }
        ?? EVStyleIcons.typeImage(role == .paragraph ? .paragraph : .character)
      if item.image !== typeImage { item.image = typeImage }
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
    if role == .character {
      let target = catalogue?.entries.first { $0.role == role && $0.stableID == targetID }
      setHidden(surface?.backend.sourceFormat.hasSameSerialization(as: .markdown) != true
        || entry == nil || target == nil, for: button)
      button.isEnabled = target?.presentation.isEnabled == true
      return
    }
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
    surface.perform(menuCommand: command, sender: sender)
    finishAction()
  }

  @objc func openImageEditor(_ sender: NSButton) { surface?.imagePopover.performToolbarAction(); refresh() }
  @objc func openReferenceEditor(_ sender: Any?) {
    surface?.linkPopover.openReferenceEditor()
  }

  @objc func openLinkEditor(_ sender: NSButton) { surface?.linkPopover.performToolbarAction(); refresh() }

  @objc func toggleCharacterCode(_ sender: NSButton) { toggleStyle(role: .character, id: "Code", fallback: "", sender: sender) }
  @objc func toggleCodeBlock(_ sender: NSButton) { toggleStyle(role: .paragraph, id: "Code Block", fallback: "Paragraph", sender: sender) }
  @objc func toggleBlockQuote(_ sender: NSButton) {
    guard let surface, let session = surface.session,
          let selected = try? session.selectedNamedStyles(), !selected.hasTable, !selected.hasCodeBlock,
          let selection = try? session.listSelection(),
          surface.backend.sourceFormat == .markdown else { refresh(); return }
    surface.performInput { _ = try session.setBlockQuote(selected.quoteState != .on, expected: selection) }
    finishAction()
  }
  /// The alert kinds, checked by the selection's current kind, with their icons.
  func makeAlertMenu() -> NSMenu {
    let menu = NSMenu(title: "Alert Type")
    menu.autoenablesItems = false
    for (title, command) in zip(["Note", "Tip", "Important", "Warning", "Caution"], EVMenuCommand.alertCommands) {
      guard let kind = command.alertKind else { continue }
      let item = NSMenuItem(title: title, action: #selector(chooseAlertItem(_:)), keyEquivalent: "")
      item.target = self
      item.tag = Int(kind)
      item.image = Self.alertImage(kind)
      let presentation = surface?.alertPresentation(kind) ?? .disabled
      item.isEnabled = presentation.isEnabled
      item.state = presentation.state
      menu.addItem(item)
    }
    return menu
  }

  @objc func showAlertMenu(_ sender: NSButton) {
    let menu = makeAlertMenu()
    // Align the menu under the whole control, starting at the quote button.
    let below = sender.isFlipped ? sender.bounds.maxY + 4 : sender.bounds.minY - 4
    menu.popUp(positioning: nil, at: NSPoint(x: blockQuote.frame.minX - sender.frame.minX, y: below), in: sender)
  }

  @objc private func chooseAlertItem(_ sender: NSMenuItem) { chooseAlert(UInt32(sender.tag)) }

  /// Choosing the selection's only alert kind again restores a plain quote.
  func chooseAlert(_ kind: UInt32) {
    guard let surface, let session = surface.session,
          let selected = try? session.selectedNamedStyles(), !selected.hasTable, !selected.hasCodeBlock,
          let selection = try? session.listSelection(),
          surface.backend.sourceFormat == .markdown else { refresh(); return }
    let target = selected.alertState(kind) == .on ? UInt32(VIEM_ALERT_NONE) : kind
    surface.performInput { _ = try session.setAlert(target, expected: selection) }
    finishAction()
  }

  static func alertImage(_ kind: UInt32) -> NSImage {
    let image = NSImage(size: NSSize(width: 16, height: 16), flipped: true) { rect in
      if let context = NSGraphicsContext.current?.cgContext {
        EVAlertDrawing.drawIcon(kind: kind, in: rect, color: NSColor.black.cgColor, context: context)
      }
      return true
    }
    image.isTemplate = true
    return image
  }

  @objc func toggleMarkdownPreview(_ sender: NSButton) {
    guard let surface else { return }
    let editorWindow = window
    surface.setMarkdownPreview(surface.backend.sourceFormat != .markdown)
    refresh()
    // The target mode may remember a hidden toolbar and detach this view.
    editorWindow?.makeFirstResponder(surface.editorView)
  }

  private func toggleStyle(role: EVStyleMenuRole, id: String, fallback: String, sender: NSButton) {
    guard let surface, let catalogue = surface.currentStyleMenuCatalogue() else { return }
    let active = catalogue.entries.contains { $0.role == role && $0.stableID == id && $0.presentation.state == .on }
    guard let entry = catalogue.entries.first(where: { $0.role == role && $0.stableID == (active ? fallback : id) }),
      entry.presentation.isEnabled else { refresh(); return }
    surface.perform(styleMenuAction: action(for: entry, in: catalogue), sender: sender)
    finishAction()
  }

  private func finishAction() {
    refresh()
    if let surface { window?.makeFirstResponder(surface.editorView) }
  }
}
