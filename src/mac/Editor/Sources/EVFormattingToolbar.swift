import AppKit
import CViemCore
import ViemAppShell

extension EVEditorSurfaceController: EVFormattingToolbarProviding {
  public var formattingToolbarFormat: EVSourceFormat { backend.sourceFormat }
  public var formattingToolbarView: NSView { formattingToolbar }
  public func refreshFormattingToolbar() { formattingToolbar.refresh() }
}

/// A presentation of the editor's existing style catalogue and document actions.
/// No selection, document formatting, or undo history is owned by these controls.
@MainActor
final class EVFormattingToolbarView: NSView, NSMenuDelegate {
  private weak var surface: EVEditorSurfaceController?
  let paragraphStyle = NSPopUpButton()
  let characterStyle = NSPopUpButton()
  private(set) var commandButtons: [EVMenuCommand: NSButton] = [:]
  let characterCode = NSButton()
  let codeBlock = NSButton()
  let blockQuote = NSButton()
  let formattedView = NSButton()
  let insertTable = EVInsertTableButton()
  let insertLink = NSButton()
  let insertImage = NSButton()
  private(set) var tablePicker: EVTablePickerController?
  private var pickerSelection: ViemLogicalSelectionIdentityV1?
  private let scroll = NSScrollView()
  private let row = NSStackView()
  private let characterGroup = NSStackView()
  private let scriptGroup = NSStackView()
  private let codeLinkGroup = NSStackView()
  private let blockGroup = NSStackView()
  private let indentGroup = NSStackView()
  private var catalogue: EVStyleMenuCatalogue?
  private var styleSnapshot: EVStyleSheetSnapshot?
  private var paragraphEntries: [String] = []
  private var characterEntries: [String] = []
  private var trackingMenus = Set<ObjectIdentifier>()

  init(surface: EVEditorSurfaceController) {
    self.surface = surface
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 42))
    setAccessibilityLabel("Formatting toolbar")
    row.orientation = .horizontal
    row.alignment = .centerY
    row.spacing = 8
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
    characterCode.action = #selector(toggleCharacterCode(_:))
    codeLinkGroup.addArrangedSubview(characterCode)
    configure(insertLink, title: "Link", symbol: "link", toggle: true)
    insertLink.action = #selector(openLinkEditor(_:))
    codeLinkGroup.addArrangedSubview(insertLink)
    add(.bulletedList, title: "Bulleted List", symbol: "list.bullet", to: blockGroup)
    add(.numberedList, title: "Numbered List", symbol: "list.number", to: blockGroup)
    configure(blockQuote, title: "Block Quote", toggle: true)
    blockQuote.image = Self.blockQuoteImage()
    blockQuote.action = #selector(toggleBlockQuote(_:))
    blockGroup.addArrangedSubview(blockQuote)
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
    row.addArrangedSubview(characterStyle)
    configure(insertTable, title: "Insert Table", toggle: false)
    insertTable.image = Self.tableImage()
    insertTable.openPicker = { [weak self] event in self?.openTablePicker(event: event) }
    scroll.drawsBackground = false
    scroll.borderType = .noBorder
    scroll.hasHorizontalScroller = true
    scroll.autohidesScrollers = true
    scroll.scrollerStyle = .overlay
    scroll.documentView = row
    addSubview(scroll)
    configure(formattedView, title: "Formatted view", toggle: true)
    formattedView.image = Self.formattedViewImage()
    formattedView.allowsMixedState = false
    formattedView.toolTip = "Formatted view (WYSIWYG)"
    formattedView.action = #selector(toggleFormattedView(_:))
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
    button.widthAnchor.constraint(equalToConstant: 27).isActive = true
    button.heightAnchor.constraint(equalToConstant: 26).isActive = true
    button.target = self
  }

  private func add(_ command: EVMenuCommand, title: String, symbol: String? = nil, to group: NSStackView, image: NSImage? = nil, toggle: Bool = true) {
    let button = NSButton()
    configure(button, title: title, symbol: symbol, toggle: toggle)
    if let image { button.image = image }
    button.tag = command.rawValue
    button.action = #selector(performCommand(_:))
    commandButtons[command] = button
    group.addArrangedSubview(button)
  }

  override func layout() {
    super.layout()
    formattedView.frame = NSRect(x: max(0, bounds.width - 37), y: (bounds.height - 26) / 2,
                                 width: 27, height: 26)
    // Keep the view toggle accessible while the formatting controls scroll.
    scroll.frame = NSRect(x: 0, y: 0, width: max(0, formattedView.frame.minX - 12), height: bounds.height)
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

  private static func formattedViewImage() -> NSImage {
    let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
      NSColor.black.setStroke()
      let page = NSBezierPath()
      page.move(to: NSPoint(x: 2.5, y: 1.5))
      page.line(to: NSPoint(x: 15.5, y: 1.5))
      page.line(to: NSPoint(x: 15.5, y: 12))
      page.line(to: NSPoint(x: 11, y: 16.5))
      page.line(to: NSPoint(x: 2.5, y: 16.5))
      page.close()
      page.move(to: NSPoint(x: 11, y: 16.5))
      page.line(to: NSPoint(x: 11, y: 12))
      page.line(to: NSPoint(x: 15.5, y: 12))
      page.lineWidth = 1
      page.stroke()
      let paragraph = NSMutableParagraphStyle()
      paragraph.alignment = .center
      ("Aa" as NSString).draw(in: NSRect(x: 3, y: 3, width: 12, height: 10), withAttributes: [
        .font: NSFont.systemFont(ofSize: 8, weight: .semibold),
        .foregroundColor: NSColor.black,
        .paragraphStyle: paragraph,
      ])
      return true
    }
    image.isTemplate = true
    return image
  }

  func refresh() {
    guard let surface else { return }
    formattedView.state = surface.backend.sourceFormat == .markdown ? .on : .off
    formattedView.isEnabled = [.markdown, .markdownSource].contains(surface.backend.sourceFormat)
    let markdown = [.markdown, .markdownSource].contains(surface.backend.sourceFormat)
    setHidden(!markdown, for: insertTable)
    setHidden(!markdown, for: insertLink)
    let link = surface.linkPopover.toolbarPresentation
    insertLink.state = link.state
    insertLink.isEnabled = markdown && link.isEnabled
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
      refresh(characterStyle, role: .character)
    }
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
    }
    refreshCode(characterCode, role: .character, id: "Code", catalogue: next)
    refreshCode(codeBlock, role: .paragraph, id: "Code Block", catalogue: next)
    blockQuote.state = selected?.quoteState ?? .off
    blockQuote.isEnabled = selected?.hasCodeBlock == false && next?.entries.contains { $0.role == .paragraph && $0.stableID == "Block quote" && $0.presentation.isEnabled } == true
    if selected?.hasTable == true {
      paragraphStyle.isEnabled = false
      for action in [EVMenuCommand.bulletedList, .numberedList, .increaseIndent, .decreaseIndent] { commandButtons[action]?.isEnabled = false }
      setHidden(true, for: codeBlock)
      blockQuote.isEnabled = false
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
    let previous = role == .paragraph ? paragraphEntries : characterEntries
    if keys != previous {
      popup.removeAllItems()
      if selected == nil {
        popup.addItem(withTitle: "Mixed")
        popup.lastItem?.isEnabled = false
        popup.lastItem?.image = EVStyleIcons.typeImage(role == .paragraph ? .paragraph : .character)
      }
      for entry in entries { popup.menu?.addItem(NSMenuItem(title: entry.displayName, action: nil, keyEquivalent: "")) }
      if role == .paragraph { paragraphEntries = keys } else { characterEntries = keys }
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
    let removeList = (command == .bulletedList || command == .numberedList)
      && surface.presentation(for: command).state == .on
    surface.perform(menuCommand: removeList ? .removeList : command, sender: sender)
    finishAction()
  }

  @objc func openImageEditor(_ sender: NSButton) { surface?.imagePopover.performToolbarAction(); refresh() }
  @objc func openLinkEditor(_ sender: NSButton) { surface?.linkPopover.performToolbarAction(); refresh() }

  @objc func toggleCharacterCode(_ sender: NSButton) { toggleStyle(role: .character, id: "Code", fallback: "", sender: sender) }
  @objc func toggleCodeBlock(_ sender: NSButton) { toggleStyle(role: .paragraph, id: "Code Block", fallback: "Paragraph", sender: sender) }
  @objc func toggleBlockQuote(_ sender: NSButton) {
    guard let surface, let session = surface.session,
          let selected = try? session.selectedNamedStyles(), !selected.hasTable, !selected.hasCodeBlock,
          let selection = try? session.listSelection(),
          surface.backend.sourceFormat == .markdown || surface.backend.sourceFormat == .markdownSource else { refresh(); return }
    surface.performInput { _ = try session.setBlockQuote(selected.quoteState != .on, expected: selection) }
    finishAction()
  }
  @objc func toggleFormattedView(_ sender: NSButton) {
    guard let surface else { return }
    let editorWindow = window
    surface.setFormattedView(surface.backend.sourceFormat != .markdown)
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
