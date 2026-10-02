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
  NSTableViewDelegate, NSTextFieldDelegate
{
  private let store: EVThemeStore
  let themeActions: EVThemeActions
  private let themeSelect = NSPopUpButton()
  private let deleteTheme = NSButton(title: "Delete Theme", target: nil, action: nil)
  private let viewPreferences: EVViewPreferences
  private var viewObserver: NSObjectProtocol?
  private let editingPreferences: EVEditingPreferences
  private let sidebar = NSTableView()
  private let content = NSView()
  private var observer: NSObjectProtocol?
  private var editingObserver: NSObjectProtocol?
  private weak var markdownAutodetectCheckbox: NSButton?
  private weak var smartQuotesCheckbox: NSButton?
  private let textWidthField = NSTextField(string: "")
  private var indentationCheckboxes: [String: NSButton] = [:]
  private var indentationFields: [String: NSTextField] = [:]
  private var whitespacePopups: [String: NSPopUpButton] = [:]
  private var listcharsFields: [String: NSTextField] = [:]
  private weak var visibleWhitespaceCheckbox: NSButton?
  private var wells: [Int: NSColorWell] = [:]
  private var fields: [Int: NSTextField] = [:]
  private let fontSelect = NSPopUpButton()
  private let preview = EVThemePreview()
  private let persistenceDiagnostic = NSTextField(wrappingLabelWithString: "")
  private var selectedCategory = 1
  private var hasPresented = false

  convenience init() { self.init(store: .shared, editingPreferences: .shared) }

  convenience init(store: EVThemeStore) { self.init(store: store, editingPreferences: .shared) }

  init(store: EVThemeStore, editingPreferences: EVEditingPreferences, viewPreferences: EVViewPreferences? = nil) {
    self.viewPreferences = viewPreferences ?? .shared
    self.store = store
    self.themeActions = EVThemeActions(store: store)
    self.editingPreferences = editingPreferences
    let window = EVSettingsWindow(
      contentRect: NSRect(x: 0, y: 0, width: 800, height: 690),
      styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
    window.title = "Viem Settings"
    window.contentMinSize = NSSize(width: 760, height: 640)
    window.isReleasedWhenClosed = false
    window.autorecalculatesKeyViewLoop = false
    super.init(window: window)
    build()
    observer = NotificationCenter.default.addObserver(
      forName: .viemThemeDidChange, object: store, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated { self?.refresh() }
    }
    viewObserver = NotificationCenter.default.addObserver(
      forName: .viemViewPreferencesDidChange, object: self.viewPreferences, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated { self?.refresh() }
    }
    editingObserver = NotificationCenter.default.addObserver(
      forName: .viemEditingPreferencesDidChange, object: nil, queue: .main
    ) { [weak self] notification in
      MainActor.assumeIsolated {
        guard let self, notification.object as AnyObject? === self.editingPreferences else { return }
        self.refreshEditingPreferences()
      }
    }
    window.setContentSize(NSSize(width: 800, height: 690))
    if window.screen != nil { window.center() }
  }

  func selectThemeCategory() {
    sidebar.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
  }

  override func showWindow(_ sender: Any?) {
    refresh()
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
    if let viewObserver { NotificationCenter.default.removeObserver(viewObserver) }
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
        systemSymbolName: ["rectangle.inset.filled", "paintpalette", "text.cursor"][row], accessibilityDescription: nil)
        ?? NSImage())
    image.contentTintColor = .secondaryLabelColor
    let label = NSTextField(labelWithString: ["View", "Theme", "Editing"][row])
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
    // End editing before removing the old page so its pending value is saved.
    window?.makeFirstResponder(sidebar)
    defer { configureKeyViewLoop() }
    content.subviews.forEach { $0.removeFromSuperview() }
    wells.removeAll()
    fields.removeAll()
    indentationCheckboxes.removeAll()
    indentationFields.removeAll()
    whitespacePopups.removeAll()
    listcharsFields.removeAll()
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
    let title = NSTextField(labelWithString: ["View", "Theme", "Editing"][selectedCategory])
    title.font = .systemFont(ofSize: 25, weight: .bold)
    stack.addArrangedSubview(title)
    let subtitle = NSTextField(
      wrappingLabelWithString: [
        "Set the space around text in every editor view.",
        "Make a comfortable space for writing. Changes apply to every window.",
        "Choose how Viem helps while you type. These preferences apply to every document.",
      ][selectedCategory])
    subtitle.textColor = .secondaryLabelColor
    stack.addArrangedSubview(subtitle)
    persistenceDiagnostic.textColor = .systemRed
    persistenceDiagnostic.font = .systemFont(ofSize: 12)
    persistenceDiagnostic.stringValue = store.lastError ?? editingPreferences.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
    stack.addArrangedSubview(persistenceDiagnostic)
    if selectedCategory == 0 {
      let margins = NSStackView()
      margins.spacing = 14
      for (index, name) in ["Top", "Left", "Bottom", "Right"].enumerated() {
        let group = NSStackView(views: [
          label(name), numberField(10 + index, "View \(name.lowercased()) margin"),
        ])
        group.orientation = .vertical
        group.alignment = .leading
        group.spacing = 5
        margins.addArrangedSubview(group)
      }
      let group = section("Text margins · pixels", views: [margins])
      stack.addArrangedSubview(group)
      group.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
      stack.addArrangedSubview(button("Restore Defaults", action: #selector(restoreViewMargins)))
      refresh()
      return
    }
    if selectedCategory == 2 {
      let checkbox = NSButton(checkboxWithTitle: "Use smart quotes", target: self, action: #selector(changeSmartQuotes(_:)))
      checkbox.state = editingPreferences.smartQuotes ? .on : .off
      checkbox.setAccessibilityLabel("Use smart quotes")
      smartQuotesCheckbox = checkbox
      let explanation = NSTextField(wrappingLabelWithString: "Use opening and closing typographic quotes for text entered or pasted into prose. Code and markup syntax keep their original quotes.")
      explanation.textColor = .secondaryLabelColor
      explanation.font = .systemFont(ofSize: 12)
      let markdown = NSButton(checkboxWithTitle: "Automatically format typed Markdown", target: self, action: #selector(changeMarkdownAutodetect(_:)))
      markdown.state = editingPreferences.markdownAutodetect ? .on : .off
      markdown.setAccessibilityLabel("Automatically format typed Markdown")
      markdownAutodetectCheckbox = markdown
      let markdownExplanation = NSTextField(wrappingLabelWithString: "In Markdown Formatted view, completed Markdown spans and block prefixes become formatting. Use Control-Q before a character to keep it literal.")
      markdownExplanation.textColor = .secondaryLabelColor
      markdownExplanation.font = .systemFont(ofSize: 12)
      let group = section("Typing assistance", views: [checkbox, explanation, markdown, markdownExplanation])
      stack.addArrangedSubview(group)
      group.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
      textWidthField.stringValue = String(editingPreferences.textWidth)
      textWidthField.alignment = .right
      textWidthField.target = self
      textWidthField.action = #selector(changeTextWidth(_:))
      textWidthField.delegate = self
      textWidthField.setAccessibilityLabel("Text width (columns)")
      textWidthField.widthAnchor.constraint(equalToConstant: 80).isActive = true
      let widthRow = NSStackView(views: [label("Text width (columns)"), textWidthField])
      widthRow.spacing = 10
      let widthExplanation = NSTextField(wrappingLabelWithString: "Columns used by gq and gw to reflow Text and Code hard lines. Open documents inherit this default unless :set textwidth overrides it. Soft wrapping and typing are unaffected.")
      widthExplanation.textColor = .secondaryLabelColor
      widthExplanation.font = .systemFont(ofSize: 12)
      let widthGroup = section("Reflow", views: [widthRow, widthExplanation])
      stack.addArrangedSubview(widthGroup)
      widthGroup.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
      buildIndentationSettings(in: stack)
      refreshEditingPreferences()
      return
    }
    themeSelect.target = self
    themeSelect.action = #selector(changeTheme(_:))
    themeSelect.setAccessibilityLabel("Theme")
    themeSelect.widthAnchor.constraint(greaterThanOrEqualToConstant: 220).isActive = true
    deleteTheme.target = self
    deleteTheme.action = #selector(removeTheme(_:))
    let themes = NSStackView(views: [label("Theme"), themeSelect,
      button("New Theme…", action: #selector(createTheme(_:))), deleteTheme])
    themes.spacing = 8
    stack.addArrangedSubview(themes)
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
    for child in stack.arrangedSubviews where child is EVThemeSettingsSection {
      child.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
    }
    refresh()
  }

  private func configureKeyViewLoop() {
    // Stack and grid order expresses the form's reading order, including values
    // below the scroll view's visible area. AppKit handles Tab/Backtab, commits
    // the field editor, and skips controls according to keyboard preferences.
    func controls(in view: NSView) -> [NSView] {
      if let field = view as? NSTextField { return field.isEditable ? [field] : [] }
      if view is NSControl { return [view] }
      if let scroll = view as? NSScrollView {
        return scroll.documentView.map { controls(in: $0) } ?? []
      }
      if let stack = view as? NSStackView { return stack.arrangedSubviews.flatMap { controls(in: $0) } }
      if let grid = view as? NSGridView {
        return (0..<grid.numberOfRows).flatMap { row in
          (0..<grid.numberOfColumns).flatMap { column in
            grid.cell(atColumnIndex: column, rowIndex: row).contentView.map { controls(in: $0) } ?? []
          }
        }
      }
      return view.subviews.flatMap { controls(in: $0) }
    }
    let keyViews = [sidebar] + controls(in: content)
    for (index, view) in keyViews.enumerated() {
      view.nextKeyView = keyViews[(index + 1) % keyViews.count]
    }
    window?.initialFirstResponder = sidebar
  }

  private func buildIndentationSettings(in stack: NSStackView) {
    let switches = [
      ("autoindent", "Copy indentation to new lines"),
      ("expandtab", "Insert spaces for tabs"),
      ("smarttab", "Use shift width for Tab in leading whitespace"),
      ("continueCommentsOnEnter", "Continue comments when pressing Return"),
      ("continueCommentsOnOpenLine", "Continue comments with o and O"),
    ].map { key, title -> NSView in
      let checkbox = NSButton(checkboxWithTitle: title, target: self, action: #selector(changeIndentationCheckbox(_:)))
      checkbox.identifier = NSUserInterfaceItemIdentifier(key)
      checkbox.setAccessibilityLabel(title)
      indentationCheckboxes[key] = checkbox
      return checkbox
    }
    let numbers = NSGridView(views: [
      [label("Tab stop"), indentationField("tabstop"), label("1–1024 columns")],
      [label("Shift width"), indentationField("shiftwidth"), label("0 uses tab stop")],
      [label("Soft tab stop"), indentationField("softtabstop"), label("−1 uses shift width; 0 disables")],
      [label("Wrapped line indent (Code)"), indentationField("codeWrappedLineIndent"), label("0–1024 Code whitespace units")],
    ])
    numbers.columnSpacing = 10
    numbers.rowSpacing = 8
    let widths = NSGridView(views: [
      [label("Code whitespace"), whitespacePopup("codeWhitespace")],
      [label("Other formats"), whitespacePopup("otherWhitespace")],
    ])
    widths.columnSpacing = 10
    widths.rowSpacing = 8
    let explanation = NSTextField(wrappingLabelWithString: "Defaults apply to new and inheriting documents. Changing these settings leaves existing text unchanged. Paragraph en uses half the paragraph font size for whitespace width. Wrapped line indent uses the Code whitespace width.")
    explanation.textColor = .secondaryLabelColor
    explanation.font = .systemFont(ofSize: 12)
    let indentation = section("Indentation and tabs", views: [numbers] + switches + [widths, explanation])
    stack.addArrangedSubview(indentation)
    indentation.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true

    let enabled = NSButton(checkboxWithTitle: "Show visible whitespace", target: self, action: #selector(changeVisibleWhitespace(_:)))
    enabled.setAccessibilityLabel("Show visible whitespace")
    visibleWhitespaceCheckbox = enabled
    let entries = NSGridView(views: EVListcharsSettings.names.map { name in
      let field = NSTextField(string: "")
      field.identifier = NSUserInterfaceItemIdentifier(name)
      field.delegate = self
      field.target = self
      field.action = #selector(changeListchars(_:))
      field.setAccessibilityLabel("Visible whitespace \(name)")
      field.widthAnchor.constraint(equalToConstant: 120).isActive = true
      listcharsFields[name] = field
      let hint = ["tab", "leadtab"].contains(name) ? "2–3 characters"
        : ["multispace", "leadmultispace"].contains(name) ? "Repeating characters" : "1 character"
      return [label(name), field, label(hint)]
    })
    entries.columnSpacing = 10
    entries.rowSpacing = 6
    let note = NSTextField(wrappingLabelWithString: "Leave a field blank to omit it and use Vim's fallback behavior. Character escapes such as \\u00b7 are supported. The Visible whitespace character style controls the markers' appearance.")
    note.textColor = .secondaryLabelColor
    note.font = .systemFont(ofSize: 12)
    let visible = section("Visible whitespace", views: [enabled, entries, note, button("Edit Style…", action: #selector(editVisibleWhitespaceStyle))])
    stack.addArrangedSubview(visible)
    visible.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -56).isActive = true
  }

  private func indentationField(_ name: String) -> NSTextField {
    let field = NSTextField(string: "")
    field.identifier = NSUserInterfaceItemIdentifier(name)
    field.alignment = .right
    field.delegate = self
    field.target = self
    field.action = #selector(changeIndentationNumber(_:))
    field.setAccessibilityLabel(name == "codeWrappedLineIndent" ? "Wrapped line indent (Code)" : "Indentation \(name)")
    field.widthAnchor.constraint(equalToConstant: 75).isActive = true
    indentationFields[name] = field
    return field
  }

  private func whitespacePopup(_ name: String) -> NSPopUpButton {
    let popup = NSPopUpButton()
    popup.identifier = NSUserInterfaceItemIdentifier(name)
    popup.addItems(withTitles: ["Use spaces", "Use paragraph en"])
    popup.target = self
    popup.action = #selector(changeWhitespaceWidth(_:))
    popup.setAccessibilityLabel(name == "codeWhitespace" ? "Code whitespace width" : "Other formats whitespace width")
    whitespacePopups[name] = popup
    return popup
  }

  private func refreshEditingPreferences(error: String? = nil) {
    markdownAutodetectCheckbox?.state = editingPreferences.markdownAutodetect ? .on : .off
    smartQuotesCheckbox?.state = editingPreferences.smartQuotes ? .on : .off
    textWidthField.stringValue = String(editingPreferences.textWidth)
    let options = editingPreferences.indentation
    let switches = ["autoindent": options.autoindent, "expandtab": options.expandtab, "smarttab": options.smarttab,
      "continueCommentsOnEnter": options.continueCommentsOnEnter, "continueCommentsOnOpenLine": options.continueCommentsOnOpenLine]
    for (key, checkbox) in indentationCheckboxes { checkbox.state = switches[key] == true ? .on : .off }
    indentationFields["tabstop"]?.stringValue = String(options.tabstop)
    indentationFields["shiftwidth"]?.stringValue = String(options.shiftwidth)
    indentationFields["softtabstop"]?.stringValue = String(options.softtabstop)
    let presentation = editingPreferences.whitespacePresentation
    indentationFields["codeWrappedLineIndent"]?.stringValue = String(presentation.codeWrappedLineIndent)
    whitespacePopups["codeWhitespace"]?.selectItem(at: presentation.codeWhitespace == .spaces ? 0 : 1)
    whitespacePopups["otherWhitespace"]?.selectItem(at: presentation.otherWhitespace == .spaces ? 0 : 1)
    visibleWhitespaceCheckbox?.state = presentation.visibleWhitespace.enabled ? .on : .off
    let entries = EVListcharsSettings.displayEntries(presentation.visibleWhitespace.listchars)
    for (key, field) in listcharsFields { field.stringValue = entries[key] ?? "" }
    if selectedCategory == 2 {
      persistenceDiagnostic.stringValue = error ?? editingPreferences.lastError ?? ""
      persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
    }
  }

  @objc private func changeIndentationCheckbox(_ sender: NSButton) {
    var options = editingPreferences.indentation
    let enabled = sender.state == .on
    switch sender.identifier?.rawValue {
    case "autoindent": options.autoindent = enabled
    case "expandtab": options.expandtab = enabled
    case "smarttab": options.smarttab = enabled
    case "continueCommentsOnEnter": options.continueCommentsOnEnter = enabled
    case "continueCommentsOnOpenLine": options.continueCommentsOnOpenLine = enabled
    default: return
    }
    editingPreferences.setIndentation(options)
    refreshEditingPreferences()
  }

  @objc private func changeIndentationNumber(_ sender: NSTextField) {
    let input = sender.stringValue.trimmingCharacters(in: .whitespaces)
    if sender.identifier?.rawValue == "codeWrappedLineIndent" {
      guard let number = Int(input), (0...1024).contains(number) else {
        refreshEditingPreferences(error: "Wrapped line indent (Code) must be a whole number from 0 to 1024.")
        return
      }
      var options = editingPreferences.whitespacePresentation
      options.codeWrappedLineIndent = number
      editingPreferences.setWhitespacePresentation(options)
      refreshEditingPreferences()
      return
    }
    var options = editingPreferences.indentation
    guard let number = Int32(input), (-1...1024).contains(number) else {
      refreshEditingPreferences(error: "Enter a whole number within the field's allowed range.")
      return
    }
    switch sender.identifier?.rawValue {
    case "tabstop" where number > 0: options.tabstop = UInt32(number)
    case "shiftwidth" where number >= 0: options.shiftwidth = UInt32(number)
    case "softtabstop": options.softtabstop = number
    default:
      refreshEditingPreferences(error: "Tab stop must be positive; shift width may be zero.")
      return
    }
    editingPreferences.setIndentation(options)
    refreshEditingPreferences()
  }

  @objc private func changeWhitespaceWidth(_ sender: NSPopUpButton) {
    var options = editingPreferences.whitespacePresentation
    let value: EVWhitespaceWidth = sender.indexOfSelectedItem == 0 ? .spaces : .paragraphEn
    if sender.identifier?.rawValue == "codeWhitespace" { options.codeWhitespace = value }
    else { options.otherWhitespace = value }
    editingPreferences.setWhitespacePresentation(options)
    refreshEditingPreferences()
  }

  @objc private func changeVisibleWhitespace(_ sender: NSButton) {
    var options = editingPreferences.whitespacePresentation
    options.visibleWhitespace.enabled = sender.state == .on
    editingPreferences.setWhitespacePresentation(options)
    refreshEditingPreferences()
  }

  @objc private func changeListchars(_ sender: NSTextField) {
    guard let name = sender.identifier?.rawValue else { return }
    var options = editingPreferences.whitespacePresentation
    guard let characters = EVListcharsSettings.decodedCharacters(sender.stringValue) else {
      refreshEditingPreferences(error: "The character escape is incomplete or invalid.")
      return
    }
    var entries = EVListcharsSettings.displayEntries(options.visibleWhitespace.listchars)
    entries[name] = characters
    options.visibleWhitespace.listchars = EVListcharsSettings.string(from: entries)
    editingPreferences.setWhitespacePresentation(options)
    refreshEditingPreferences()
  }

  @objc private func editVisibleWhitespaceStyle() { editingPreferences.openVisibleWhitespaceStyle() }

  func controlTextDidEndEditing(_ notification: Notification) {
    if notification.object as AnyObject? === textWidthField { changeTextWidth(textWidthField) }
    if let field = notification.object as? NSTextField, let name = field.identifier?.rawValue {
      if indentationFields[name] === field { changeIndentationNumber(field) }
      if listcharsFields[name] === field { changeListchars(field) }
    }
    if let field = notification.object as? NSTextField, fields[field.tag] === field { changeNumber(field) }
  }

  /// Zero, negative, fractional, malformed, and overflowing values are
  /// rejected; the field reverts to the persisted default.
  @objc private func changeTextWidth(_ sender: NSTextField) {
    let text = sender.stringValue.trimmingCharacters(in: .whitespaces)
    if !text.isEmpty, text.allSatisfy(\.isNumber), let width = UInt32(text), width > 0 {
      editingPreferences.setTextWidth(width)
    }
    sender.stringValue = String(editingPreferences.textWidth)
    persistenceDiagnostic.stringValue = editingPreferences.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
  }

  func showViewCategoryForTesting() {
    sidebar.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
    selectedCategory = 0
    showCategory()
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
    field.delegate = self
    field.action = #selector(changeNumber(_:))
    field.setAccessibilityLabel(name)
    field.widthAnchor.constraint(equalToConstant: 60).isActive = true
    fields[tag] = field
    return field
  }
  private func refresh() {
    try? store.ensureCurrentThemeExists()
    persistenceDiagnostic.stringValue = (selectedCategory == 0 ? viewPreferences.lastError : store.lastError) ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
    themeSelect.removeAllItems()
    for theme in store.availableThemes {
      let item = NSMenuItem(title: theme.name, action: nil, keyEquivalent: "")
      item.representedObject = theme
      themeSelect.menu?.addItem(item)
    }
    themeSelect.menu?.addItem(.separator())
    themeSelect.menu?.addItem(NSMenuItem(title: "Default", action: nil, keyEquivalent: ""))
    themeSelect.select(themeSelect.itemArray.first {
      !$0.isSeparatorItem && ($0.representedObject as? EVThemeChoice)?.fileName == store.currentThemeFileName
    })
    deleteTheme.isEnabled = store.currentThemeName != nil
    let theme = store.theme
    let colors = [
      theme.foreground, theme.background, theme.caret, theme.selection, theme.statusForeground,
      theme.statusBackground,
    ]
    for (tag, well) in wells { well.color = colors[tag].color }
    fields[6]?.doubleValue = theme.statusFontSize
    let margins = viewPreferences.margins
    for (index, value) in [
      margins.top, margins.left, margins.bottom, margins.right,
    ].enumerated() { fields[10 + index]?.doubleValue = value }
    fontSelect.selectItem(withTitle: theme.statusFontFamily)
    preview.theme = theme
  }
  @objc private func restoreViewMargins() {
    viewPreferences.setMargins(EVViewMargins())
    refresh()
  }
  @objc private func changeTheme(_ sender: NSPopUpButton) {
    themeActions.select(sender.selectedItem?.representedObject as? EVThemeChoice)
    refresh()
  }
  @objc private func createTheme(_ sender: Any?) {
    themeActions.create(window: window)
    refresh()
  }
  @objc private func removeTheme(_ sender: Any?) {
    themeActions.delete()
    refresh()
  }
  @objc private func changeMarkdownAutodetect(_ sender: NSButton) {
    editingPreferences.setMarkdownAutodetect(sender.state == .on)
    refreshEditingPreferences()
  }
  @objc private func changeSmartQuotes(_ sender: NSButton) {
    editingPreferences.setSmartQuotes(sender.state == .on)
    sender.state = editingPreferences.smartQuotes ? .on : .off
    persistenceDiagnostic.stringValue = editingPreferences.lastError ?? ""
    persistenceDiagnostic.isHidden = persistenceDiagnostic.stringValue.isEmpty
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
    if sender.tag == 6 {
      var theme = store.theme
      theme.statusFontSize = value
      store.update(theme)
    } else {
      var margins = viewPreferences.margins
      switch sender.tag {
      case 10: margins.top = value
      case 11: margins.left = value
      case 12: margins.bottom = value
      default: margins.right = value
      }
      viewPreferences.setMargins(margins)
    }
    refresh()
  }
}

@MainActor
private final class EVThemePreview: NSView {
  var theme = EVTheme.midnight { didSet { needsDisplay = true } }
  override var isFlipped: Bool { true }
  override func draw(_ dirtyRect: NSRect) {
    NSBezierPath(roundedRect: bounds, xRadius: 9, yRadius: 9).addClip()
    theme.background.color.setFill()
    bounds.fill()
    let x = min(CGFloat(EVViewMargins().left) * 0.55 + 8, 95)
    let y = min(CGFloat(EVViewMargins().top) * 0.55 + 7, 44)
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
    ("NORMAL     ◉ Ln 1, Col 1     UTF-8    Markdown" as NSString).draw(
      at: NSPoint(x: 12, y: status.minY + 6),
      withAttributes: [.font: theme.statusFont, .foregroundColor: theme.statusForeground.color])
  }
}
