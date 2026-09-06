import AppKit

private final class EVSettingsWindow: NSWindow {
  override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
    guard let screen else { return frameRect }
    return super.constrainFrameRect(frameRect, to: screen)
  }
}

private final class EVThemeSettingsStack: NSStackView {
  override var isFlipped: Bool { true }
}

private final class EVThemeSettingsSection: NSView {}

@MainActor
final class EVSettingsWindowController: NSWindowController, NSTableViewDataSource,
  NSTableViewDelegate
{
  private let store: EVThemeStore
  private let editingPreferences: EVEditingPreferences
  private let sidebar = NSTableView()
  private let content = NSView()
  private var observer: NSObjectProtocol?
  private var editingObserver: NSObjectProtocol?
  private weak var smartQuotesCheckbox: NSButton?
  private var wells: [Int: NSColorWell] = [:]
  private var fields: [Int: NSTextField] = [:]
  private let fontSelect = NSPopUpButton()
  private let preview = EVThemePreview()
  private let persistenceDiagnostic = NSTextField(wrappingLabelWithString: "")
  private var selectedCategory = 1
  private var hasPresented = false

  convenience init() { self.init(store: .shared, editingPreferences: .shared) }

  convenience init(store: EVThemeStore) { self.init(store: store, editingPreferences: .shared) }

  init(store: EVThemeStore, editingPreferences: EVEditingPreferences) {
    self.store = store
    self.editingPreferences = editingPreferences
    let window = EVSettingsWindow(
      contentRect: NSRect(x: 0, y: 0, width: 800, height: 690),
      styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
    window.title = "eVim Settings"
    window.contentMinSize = NSSize(width: 760, height: 640)
    window.isReleasedWhenClosed = false
    super.init(window: window)
    build()
    observer = NotificationCenter.default.addObserver(
      forName: .evimThemeDidChange, object: store, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated { self?.refresh() }
    }
    editingObserver = NotificationCenter.default.addObserver(
      forName: .evimEditingPreferencesDidChange, object: nil, queue: .main
    ) { [weak self] notification in
      MainActor.assumeIsolated {
        guard let self, notification.object as AnyObject? === self.editingPreferences else { return }
        self.smartQuotesCheckbox?.state = self.editingPreferences.smartQuotes ? .on : .off
      }
    }
    window.setContentSize(NSSize(width: 800, height: 690))
    if window.screen != nil { window.center() }
  }

  override func showWindow(_ sender: Any?) {
    if !hasPresented { window?.setContentSize(NSSize(width: 800, height: 690)) }
    super.showWindow(sender)
    if !hasPresented {
      window?.setContentSize(NSSize(width: 800, height: 690))
      window?.center()
      hasPresented = true
    }
  }

  deinit {
    if let observer { NotificationCenter.default.removeObserver(observer) }
    if let editingObserver { NotificationCenter.default.removeObserver(editingObserver) }
  }
  @available(*, unavailable) required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  private func build() {
    let root = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 690))
    root.translatesAutoresizingMaskIntoConstraints = false
    root.autoresizingMask = [.width, .height]
    let rail = NSVisualEffectView()
    rail.material = .sidebar
    rail.blendingMode = .behindWindow
    let heading = NSTextField(labelWithString: "SETTINGS")
    heading.font = .systemFont(ofSize: 10, weight: .semibold)
    heading.textColor = .secondaryLabelColor
    let column = NSTableColumn(identifier: .init("category"))
    sidebar.addTableColumn(column)
    sidebar.headerView = nil
    sidebar.backgroundColor = .clear
    sidebar.rowHeight = 36
    sidebar.style = .sourceList
    sidebar.focusRingType = .none
    sidebar.dataSource = self
    sidebar.delegate = self
    sidebar.setAccessibilityLabel("Settings categories")
    let navigation = NSScrollView()
    navigation.drawsBackground = false
    navigation.hasVerticalScroller = false
    navigation.borderType = .noBorder
    sidebar.frame = NSRect(x: 0, y: 0, width: 148, height: 120)
    sidebar.autoresizingMask = [.width]
    navigation.documentView = sidebar
    rail.addSubview(heading)
    rail.addSubview(navigation)
    root.addSubview(rail)
    root.addSubview(content)
    for view in [rail, heading, navigation, content] {
      view.translatesAutoresizingMaskIntoConstraints = false
    }
    NSLayoutConstraint.activate([
      root.widthAnchor.constraint(greaterThanOrEqualToConstant: 760),
      root.heightAnchor.constraint(greaterThanOrEqualToConstant: 640),
      rail.leadingAnchor.constraint(equalTo: root.leadingAnchor),
      rail.topAnchor.constraint(equalTo: root.topAnchor),
      rail.bottomAnchor.constraint(equalTo: root.bottomAnchor),
      rail.widthAnchor.constraint(equalToConstant: 168),
      heading.leadingAnchor.constraint(equalTo: rail.leadingAnchor, constant: 20),
      heading.topAnchor.constraint(equalTo: rail.topAnchor, constant: 25),
      navigation.leadingAnchor.constraint(equalTo: rail.leadingAnchor, constant: 10),
      navigation.trailingAnchor.constraint(equalTo: rail.trailingAnchor, constant: -10),
      navigation.topAnchor.constraint(equalTo: heading.bottomAnchor, constant: 12),
      navigation.bottomAnchor.constraint(equalTo: rail.bottomAnchor, constant: -16),
      content.leadingAnchor.constraint(equalTo: rail.trailingAnchor),
      content.trailingAnchor.constraint(equalTo: root.trailingAnchor),
      content.topAnchor.constraint(equalTo: root.topAnchor),
      content.bottomAnchor.constraint(equalTo: root.bottomAnchor),
    ])
    window?.contentView = root
    sidebar.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
    showCategory()
  }

  func numberOfRows(in tableView: NSTableView) -> Int { 3 }
  func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView?
  {
    let image = NSImageView(
      image: NSImage(
        systemSymbolName: ["doc.text", "paintpalette", "text.cursor"][row], accessibilityDescription: nil)
        ?? NSImage())
    image.contentTintColor = .secondaryLabelColor
    let label = NSTextField(labelWithString: ["Documents", "Theme", "Editing"][row])
    label.font = .systemFont(ofSize: 13, weight: .medium)
    let row = NSStackView(views: [image, label])
    row.spacing = 9
    row.alignment = .centerY
    image.widthAnchor.constraint(equalToConstant: 18).isActive = true
    return row
  }
  func tableViewSelectionDidChange(_ notification: Notification) {
    guard sidebar.selectedRow >= 0 else { return }
    selectedCategory = sidebar.selectedRow
    showCategory()
  }

  private func showCategory() {
    content.subviews.forEach { $0.removeFromSuperview() }
    wells.removeAll()
    fields.removeAll()
    let scroll = NSScrollView()
    scroll.drawsBackground = false
    scroll.hasVerticalScroller = true
    scroll.translatesAutoresizingMaskIntoConstraints = false
    content.addSubview(scroll)
    NSLayoutConstraint.activate([
      scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
      scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
      scroll.topAnchor.constraint(equalTo: content.topAnchor),
      scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
    ])
    let stack = EVThemeSettingsStack()
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 18
    stack.edgeInsets = NSEdgeInsets(top: 25, left: 28, bottom: 24, right: 28)
    stack.translatesAutoresizingMaskIntoConstraints = false
    scroll.documentView = stack
    stack.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor).isActive = true
    let title = NSTextField(labelWithString: ["Documents", "Theme", "Editing"][selectedCategory])
    title.font = .systemFont(ofSize: 25, weight: .bold)
    stack.addArrangedSubview(title)
    let subtitle = NSTextField(
      wrappingLabelWithString: [
        "Default styles and app settings are saved in ~/.evim. Documents can override their format’s defaults.",
        "Make a comfortable space for writing. Changes apply to every window.",
        "Choose how eVim helps while you type. These preferences apply to every document.",
      ][selectedCategory])
    subtitle.textColor = .secondaryLabelColor
    stack.addArrangedSubview(subtitle)
    persistenceDiagnostic.textColor = .systemRed
    persistenceDiagnostic.font = .systemFont(ofSize: 12)
    persistenceDiagnostic.stringValue = store.lastError ?? editingPreferences.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
    stack.addArrangedSubview(persistenceDiagnostic)
    if selectedCategory == 0 {
      stack.addArrangedSubview(
        section(
          "New documents",
          views: [
            label("Text, HTML, Markdown, and RTF each have their own default style set."),
            label("Use Format > Style to edit a document’s styles or save them as its format’s defaults."),
          ]))
      return
    }
    if selectedCategory == 2 {
      let checkbox = NSButton(checkboxWithTitle: "Use smart quotes", target: self, action: #selector(changeSmartQuotes(_:)))
      checkbox.state = editingPreferences.smartQuotes ? .on : .off
      checkbox.setAccessibilityLabel("Use smart quotes")
      smartQuotesCheckbox = checkbox
      let explanation = NSTextField(wrappingLabelWithString: "Use opening and closing typographic quotes in prose. Quotes required by markup keep their original spelling.")
      explanation.textColor = .secondaryLabelColor
      explanation.font = .systemFont(ofSize: 12)
      let group = section("Typing assistance", views: [checkbox, explanation])
      stack.addArrangedSubview(group)
      group.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
      return
    }
    let paper = button("Paper", action: #selector(usePaper))
    let midnight = button("Midnight", action: #selector(useMidnight))
    let presets = NSStackView(views: [paper, midnight])
    presets.spacing = 8
    stack.addArrangedSubview(presets)
    preview.translatesAutoresizingMaskIntoConstraints = false
    preview.heightAnchor.constraint(equalToConstant: 135).isActive = true
    stack.addArrangedSubview(preview)
    preview.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
    let colors = NSGridView(views: [
      [
        label("Text"), colorWell(0, "Theme text color"), label("Canvas"),
        colorWell(1, "Theme canvas color"),
      ],
      [
        label("Caret"), colorWell(2, "Theme caret color"), label("Selection"),
        colorWell(3, "Theme selection color"),
      ],
    ])
    colors.columnSpacing = 14
    colors.rowSpacing = 10
    stack.addArrangedSubview(section("Editor", views: [colors]))
    let statusColors = NSGridView(views: [
      [
        label("Text"), colorWell(4, "Status text color"), label("Background"),
        colorWell(5, "Status background color"),
      ]
    ])
    statusColors.columnSpacing = 14
    fontSelect.removeAllItems()
    fontSelect.addItems(
      withTitles: ["System"] + NSFontManager.shared.availableFontFamilies.sorted())
    fontSelect.target = self
    fontSelect.action = #selector(changeFont)
    fontSelect.setAccessibilityLabel("Status font family")
    fontSelect.widthAnchor.constraint(equalToConstant: 210).isActive = true
    let fonts = NSStackView(views: [
      label("Font"), fontSelect, numberField(6, "Status font size"), label("pt"),
    ])
    fonts.spacing = 10
    stack.addArrangedSubview(section("Status bar", views: [statusColors, fonts]))
    let pads = NSStackView()
    pads.spacing = 14
    for (index, name) in ["Top", "Left", "Bottom", "Right"].enumerated() {
      let group = NSStackView(views: [
        label(name), numberField(10 + index, "Document \(name.lowercased()) padding"),
      ])
      group.orientation = .vertical
      group.alignment = .leading
      group.spacing = 5
      pads.addArrangedSubview(group)
    }
    let note = label("Padding moves with the document as you scroll.")
    note.font = .systemFont(ofSize: 11)
    note.textColor = .secondaryLabelColor
    stack.addArrangedSubview(section("Document padding · pt", views: [pads, note]))
    let reset = button("Restore Defaults", action: #selector(usePaper))
    reset.controlSize = .small
    stack.addArrangedSubview(reset)
    for child in stack.arrangedSubviews where child is EVThemeSettingsSection {
      child.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
    }
    refresh()
  }

  private func label(_ text: String) -> NSTextField { NSTextField(labelWithString: text) }
  private func section(_ title: String, views: [NSView]) -> NSView {
    let box = EVThemeSettingsSection()
    box.wantsLayer = true
    box.layer?.borderColor = NSColor.separatorColor.cgColor
    box.layer?.borderWidth = 0.5
    box.layer?.cornerRadius = 9
    let heading = label(title)
    heading.font = .systemFont(ofSize: 12, weight: .semibold)
    let stack = NSStackView(views: [heading] + views)
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 12
    stack.translatesAutoresizingMaskIntoConstraints = false
    box.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 15),
      stack.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -15),
      stack.topAnchor.constraint(equalTo: box.topAnchor, constant: 12),
      stack.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -12),
    ])
    return box
  }
  private func button(_ title: String, action: Selector) -> NSButton {
    NSButton(title: title, target: self, action: action)
  }
  private func colorWell(_ tag: Int, _ name: String) -> NSColorWell {
    let well = NSColorWell()
    well.colorWellStyle = .minimal
    well.tag = tag
    well.target = self
    well.action = #selector(changeColor(_:))
    well.setAccessibilityLabel(name)
    well.widthAnchor.constraint(equalToConstant: 70).isActive = true
    well.heightAnchor.constraint(equalToConstant: 26).isActive = true
    wells[tag] = well
    return well
  }
  private func numberField(_ tag: Int, _ name: String) -> NSTextField {
    let field = NSTextField(string: "")
    field.tag = tag
    field.alignment = .right
    field.target = self
    field.action = #selector(changeNumber(_:))
    field.setAccessibilityLabel(name)
    field.widthAnchor.constraint(equalToConstant: 60).isActive = true
    fields[tag] = field
    return field
  }
  private func refresh() {
    persistenceDiagnostic.stringValue = store.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
    let theme = store.theme
    let colors = [
      theme.foreground, theme.background, theme.caret, theme.selection, theme.statusForeground,
      theme.statusBackground,
    ]
    for (tag, well) in wells { well.color = colors[tag].color }
    fields[6]?.doubleValue = theme.statusFontSize
    for (index, value) in [
      theme.padding.top, theme.padding.left, theme.padding.bottom, theme.padding.right,
    ].enumerated() { fields[10 + index]?.doubleValue = value }
    fontSelect.selectItem(withTitle: theme.statusFontFamily)
    preview.theme = theme
  }
  @objc private func usePaper() {
    store.update(.paper)
    refresh()
  }
  @objc private func changeSmartQuotes(_ sender: NSButton) {
    editingPreferences.setSmartQuotes(sender.state == .on)
    sender.state = editingPreferences.smartQuotes ? .on : .off
    persistenceDiagnostic.stringValue = editingPreferences.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
  }
  @objc private func useMidnight() {
    store.update(.midnight)
    refresh()
  }
  @objc private func changeFont() {
    var theme = store.theme
    theme.statusFontFamily = fontSelect.titleOfSelectedItem ?? "System"
    store.update(theme)
  }
  @objc private func changeColor(_ sender: NSColorWell) {
    var theme = store.theme
    let color = EVThemeColor(sender.color)
    switch sender.tag {
    case 0: theme.foreground = color
    case 1: theme.background = color
    case 2: theme.caret = color
    case 3: theme.selection = color
    case 4: theme.statusForeground = color
    default: theme.statusBackground = color
    }
    store.update(theme)
  }
  @objc private func changeNumber(_ sender: NSTextField) {
    guard let value = Double(sender.stringValue), value.isFinite else {
      refresh()
      return
    }
    var theme = store.theme
    switch sender.tag {
    case 6: theme.statusFontSize = value
    case 10: theme.padding.top = value
    case 11: theme.padding.left = value
    case 12: theme.padding.bottom = value
    default: theme.padding.right = value
    }
    store.update(theme)
    refresh()
  }
}

@MainActor
private final class EVThemePreview: NSView {
  var theme = EVTheme.paper { didSet { needsDisplay = true } }
  override var isFlipped: Bool { true }
  override func draw(_ dirtyRect: NSRect) {
    NSBezierPath(roundedRect: bounds, xRadius: 9, yRadius: 9).addClip()
    theme.background.color.setFill()
    bounds.fill()
    let x = min(CGFloat(theme.padding.left) * 0.55 + 8, 95)
    let y = min(CGFloat(theme.padding.top) * 0.55 + 7, 44)
    let font = NSFont.systemFont(ofSize: 18, weight: .medium)
    ("A space for your words." as NSString).draw(
      at: NSPoint(x: x, y: y),
      withAttributes: [.font: font, .foregroundColor: theme.foreground.color])
    let selection = NSRect(x: x, y: y + 33, width: 150, height: 19)
    theme.selection.color.setFill()
    selection.fill()
    ("Keep your own rhythm." as NSString).draw(
      at: NSPoint(x: x, y: y + 32),
      withAttributes: [
        .font: NSFont.systemFont(ofSize: 13), .foregroundColor: theme.foreground.color,
      ])
    theme.caret.color.setFill()
    NSRect(x: x + 195, y: y, width: 11, height: 22).fill()
    let status = NSRect(x: 0, y: bounds.height - 26, width: bounds.width, height: 26)
    theme.statusBackground.color.setFill()
    status.fill()
    ("NORMAL     ◉ Ln 1, Col 1     UTF-8    HTML" as NSString).draw(
      at: NSPoint(x: 12, y: status.minY + 6),
      withAttributes: [.font: theme.statusFont, .foregroundColor: theme.statusForeground.color])
  }
}
