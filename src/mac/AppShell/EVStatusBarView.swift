import AppKit

@MainActor
public final class EVStatusBarView: NSView {
  public static var preferredHeight: CGFloat {
    max(25, ceil(EVThemeStore.shared.theme.statusFontSize + 14))
  }

  /// The platform's standard window corner radius. AppKit exposes no API for
  /// it, so it is stated here; the inset policy below is what matters.
  public static let windowCornerRadius: CGFloat = 16

  /// Width of the strip a rounded corner clips by more than one point.
  ///
  /// At horizontal distance `x` from the edge, a corner of radius `r` has
  /// removed `r - sqrt(r² - (r - x)²)` of height. Solving that for one point
  /// gives `x = r - sqrt(2r - 1)`. Insetting by it keeps text clear of the
  /// curve without wasting the whole radius.
  public static func cornerInset(forRadius radius: CGFloat) -> CGFloat {
    guard radius > 1 else { return 0 }
    return max(0, radius - (2 * radius - 1).squareRoot())
  }

  /// Every status line uses the same inset, including panes in the middle of a
  /// window whose corners are square, so that all of them align.
  public static var contentInset: CGFloat { cornerInset(forRadius: windowCornerRadius) }

  /// Space between the command area and the caret position widget.
  static let commandAreaGap: CGFloat = 5

  public var preferredHeightDidChange: (() -> Void)?
  private var themeObserver: NSObjectProtocol?
  private var heightConstraint: NSLayoutConstraint!
  private var currentState = EVStatusBarState()

  private let modeLabel = NSTextField(labelWithString: "")
  private let messageLabel = NSTextField(labelWithString: "")
  private let locationLabel = NSButton(title: "", target: nil, action: nil)
  public var optionDidChange: ((EVStatusBarOption) -> Void)?
  /// A click inside the command area, as a UTF-8 offset into its text.
  public var commandLineDidSelect: ((Int, Bool) -> Void)?
  public var commandOutputDidDismiss: (() -> Void)?
  public var commandOutputDidReceiveKey: ((NSEvent) -> Void)?
  private let formatSelect = EVStatusSelect()
  private let formatLabel = NSTextField(labelWithString: "")
  private let leftGroup = NSStackView()
  private let commandCaret = NSTextInsertionIndicator(frame: .zero)
  private let outputScroll = NSScrollView()
  let outputTextView = EVStatusOutputTextView()
  private let outputCloseButton = NSButton()

  public override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    translatesAutoresizingMaskIntoConstraints = false
    setAccessibilityRole(.group)
    setAccessibilityLabel("Editor status")

    let separator = NSBox()
    separator.boxType = .separator
    separator.translatesAutoresizingMaskIntoConstraints = false

    wantsLayer = true
    locationLabel.isBordered = false
    locationLabel.imagePosition = .imageLeading
    locationLabel.imageScaling = .scaleProportionallyDown
    locationLabel.target = self
    locationLabel.action = #selector(toggleLineMode)
    locationLabel.setContentHuggingPriority(.required, for: .horizontal)
    modeLabel.font = .systemFont(ofSize: 11, weight: .semibold)
    modeLabel.setContentHuggingPriority(.required, for: .horizontal)
    messageLabel.textColor = .secondaryLabelColor
    messageLabel.lineBreakMode = .byTruncatingTail
    messageLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

    formatSelect.configure(
      label: "Format",
      options: [EVSourceFormat.markdownSource, .markdown].map {
        ($0.displayName, .format($0))
      }
    )
    formatSelect.didChoose = { [weak self] option in self?.optionDidChange?(option) }
    // Everything except the caret position widget is left-aligned; the widget
    // is always present and always at the trailing edge.
    leftGroup.setViews([modeLabel, formatSelect, formatLabel, messageLabel], in: .leading)
    leftGroup.orientation = .horizontal
    leftGroup.spacing = 14
    leftGroup.alignment = .centerY
    leftGroup.translatesAutoresizingMaskIntoConstraints = false
    locationLabel.translatesAutoresizingMaskIntoConstraints = false
    commandCaret.displayMode = .hidden
    commandCaret.isHidden = true
    commandCaret.automaticModeOptions = [.showEffectsView, .showWhileTracking]

    addSubview(separator)
    addSubview(leftGroup)
    addSubview(locationLabel)
    addSubview(commandCaret)
    outputTextView.isEditable = false
    outputTextView.isSelectable = true
    outputTextView.isRichText = false
    outputTextView.drawsBackground = false
    outputTextView.textContainerInset = .zero
    outputTextView.textContainer?.lineFragmentPadding = 0
    outputTextView.textContainer?.widthTracksTextView = false
    outputTextView.textContainer?.containerSize = NSSize(
      width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
    outputTextView.isHorizontallyResizable = true
    outputTextView.isVerticallyResizable = true
    outputTextView.maxSize = NSSize(
      width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
    outputTextView.didReceiveEditorKey = { [weak self] event in
      self?.commandOutputDidReceiveKey?(event)
    }
    outputTextView.setAccessibilityLabel("Command output")
    outputScroll.documentView = outputTextView
    outputScroll.drawsBackground = false
    outputScroll.borderType = .noBorder
    outputScroll.hasHorizontalScroller = false
    outputScroll.hasVerticalScroller = false
    addSubview(outputScroll)
    outputCloseButton.image = NSImage(
      systemSymbolName: "xmark", accessibilityDescription: "Close command output")
    outputCloseButton.imagePosition = .imageOnly
    outputCloseButton.isBordered = false
    outputCloseButton.target = self
    outputCloseButton.action = #selector(dismissCommandOutput(_:))
    outputCloseButton.setAccessibilityLabel("Close command output")
    addSubview(outputCloseButton)
    heightConstraint = heightAnchor.constraint(equalToConstant: Self.preferredHeight)
    let inset = Self.contentInset
    NSLayoutConstraint.activate([
      heightConstraint,
      separator.leadingAnchor.constraint(equalTo: leadingAnchor),
      separator.trailingAnchor.constraint(equalTo: trailingAnchor),
      separator.topAnchor.constraint(equalTo: topAnchor),
      leftGroup.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset),
      leftGroup.topAnchor.constraint(equalTo: separator.bottomAnchor),
      leftGroup.bottomAnchor.constraint(equalTo: bottomAnchor),
      leftGroup.trailingAnchor.constraint(
        lessThanOrEqualTo: locationLabel.leadingAnchor, constant: -14),
      locationLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset),
      locationLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
    ])

    themeObserver = NotificationCenter.default.addObserver(
      forName: .viemThemeDidChange, object: EVThemeStore.shared, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated {
        self?.applyTheme()
        self?.preferredHeightDidChange?()
      }
    }
    apply(EVStatusBarState())
  }

  deinit { if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) } }

  private func applyTheme() {
    let theme = EVThemeStore.shared.theme
    layer?.backgroundColor = theme.statusBackground.color.cgColor
    modeLabel.font = theme.statusFont
    messageLabel.font = theme.statusFont
    modeLabel.textColor = theme.statusForeground.color
    messageLabel.textColor = theme.statusForeground.color.withAlphaComponent(0.75)
    locationLabel.font = theme.statusFont
    locationLabel.contentTintColor = theme.statusForeground.color
    locationLabel.attributedTitle = NSAttributedString(
      string: currentState.location,
      attributes: [.font: theme.statusFont, .foregroundColor: theme.statusForeground.color])
    formatSelect.applyTheme(theme)
    formatLabel.font = theme.statusFont
    formatLabel.textColor = theme.statusForeground.color
    outputTextView.font = commandFont
    outputTextView.textColor = theme.statusForeground.color
    outputTextView.insertionPointColor = .clear
    outputCloseButton.contentTintColor = theme.statusForeground.color
    heightConstraint.constant = Self.preferredHeight
    needsDisplay = true
  }

  @objc private func toggleLineMode() {
    optionDidChange?(.lineMode(currentState.lineMode == .visual ? .physicalSource : .visual))
  }

  @objc private func dismissCommandOutput(_ sender: Any?) { commandOutputDidDismiss?() }

  public var isCommandOutputFocused: Bool {
    !outputScroll.isHidden && window?.firstResponder === outputTextView
  }

  /// Native menu commands must copy the selected message, even though the
  /// application menu normally routes editor commands through the core.
  public func commandOutputPresentation(for command: EVMenuCommand) -> EVMenuItemPresentation? {
    guard isCommandOutputFocused else { return nil }
    switch command {
    case .copy, .copySource:
      return EVMenuItemPresentation(isEnabled: outputTextView.selectedRange().length > 0)
    case .selectAll: return .enabled
    case .cut, .delete: return .disabled
    default: return nil
    }
  }

  public func performCommandOutputAction(_ command: EVMenuCommand) -> Bool {
    guard isCommandOutputFocused else { return false }
    switch command {
    case .copy, .copySource: outputTextView.copy(nil)
    case .selectAll: outputTextView.selectAll(nil)
    case .cut, .delete: break // Output is read-only, including menu actions.
    default: return false
    }
    return true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  /// Monospaced so a typed command lines up with what the core measures.
  private var commandFont: NSFont {
    .monospacedSystemFont(ofSize: EVThemeStore.shared.theme.statusFontSize, weight: .regular)
  }

  /// The command area runs from the leading window edge to the caret position
  /// widget, less the gap. Its background reaches the edge; its text does not.
  var commandAreaRect: NSRect {
    let limit = max(0, locationLabel.frame.minX - Self.commandAreaGap)
    return NSRect(x: 0, y: 0, width: limit, height: bounds.height)
  }

  /// Horizontal offset of one UTF-8 position within the drawn command text,
  /// before scrolling.
  private func commandTextX(upTo utf8Offset: Int, in command: EVStatusCommandLine) -> CGFloat {
    let text = command.displayText
    let limit = command.prompt.utf8.count + max(0, utf8Offset)
    guard let prefix = String(bytes: Array(text.utf8.prefix(limit)), encoding: .utf8) else {
      return 0
    }
    return (prefix as NSString).size(withAttributes: [.font: commandFont]).width
  }

  /// Scroll applied so the caret stays inside the command area.
  private func commandScroll(_ command: EVStatusCommandLine) -> CGFloat {
    let caret = commandTextX(upTo: command.cursorUTF8Offset, in: command)
    let inset = Self.contentInset
    let available = max(0, commandAreaRect.width - inset - 2)
    return max(0, caret - available)
  }

  /// The UTF-8 offset nearest a point in this view, for pointer selection.
  func commandUTF8Offset(at point: NSPoint) -> Int? {
    guard let command = currentState.commandLine else { return nil }
    let origin = Self.contentInset - commandScroll(command)
    var best = 0
    var distance = CGFloat.greatestFiniteMagnitude
    var offset = 0
    for character in Array(command.text) + ["\u{0}"] {
      let x = origin + commandTextX(upTo: offset, in: command)
      if abs(x - point.x) < distance {
        distance = abs(x - point.x)
        best = offset
      }
      offset += String(character).utf8.count
    }
    return best
  }

  public override func mouseDown(with event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    guard currentState.commandLine != nil, commandAreaRect.contains(point),
      let offset = commandUTF8Offset(at: point)
    else {
      super.mouseDown(with: event)
      return
    }
    commandLineDidSelect?(offset, event.modifierFlags.contains(.shift))
  }

  public override func draw(_ dirtyRect: NSRect) {
    super.draw(dirtyRect)
    guard let command = currentState.commandLine else { return }
    let theme = EVThemeStore.shared.theme
    // Inverse of the rest of the status line, and the background runs all the
    // way to the leading window edge even though the text is inset.
    let background = theme.statusForeground.color
    let foreground = theme.statusBackground.color
    let area = commandAreaRect
    background.setFill()
    area.fill()

    let font = commandFont
    let text = command.displayText
    let rendered = NSMutableAttributedString(
      string: text, attributes: [.font: font, .foregroundColor: foreground])
    let length = rendered.length
    if let marked = command.markedDisplayRange, NSMaxRange(marked) <= length {
      rendered.addAttributes(
        [.underlineStyle: NSUnderlineStyle.single.rawValue, .underlineColor: foreground],
        range: marked)
    }
    if let selected = command.selectedDisplayRange, selected.length > 0,
      NSMaxRange(selected) <= length
    {
      rendered.addAttribute(.backgroundColor, value: foreground.withAlphaComponent(0.3), range: selected)
    }
    let lineHeight = ceil(font.ascender - font.descender + font.leading)
    let origin = NSPoint(
      x: Self.contentInset - commandScroll(command),
      y: floor((bounds.height - lineHeight) / 2))
    NSGraphicsContext.saveGraphicsState()
    NSBezierPath(rect: NSRect(
      x: Self.contentInset, y: 0,
      width: max(0, area.width - Self.contentInset), height: area.height)).setClip()
    rendered.draw(at: origin)
    // Preserve the thin command caret's geometry when outlining it steadily.
    if !currentState.isActive {
      let outline = commandCaretFrame(command)
      foreground.withAlphaComponent(0.75).setStroke()
      NSBezierPath(rect: outline.insetBy(dx: 0.5, dy: 0.5)).stroke()
    }
    NSGraphicsContext.restoreGraphicsState()
  }

  /// Caret rectangle for the command line, in this view's coordinates.
  private func commandCaretFrame(_ command: EVStatusCommandLine) -> NSRect {
    let font = commandFont
    let lineHeight = ceil(font.ascender - font.descender + font.leading)
    let x = Self.contentInset - commandScroll(command)
      + commandTextX(upTo: command.cursorUTF8Offset, in: command)
    return NSRect(
      x: x, y: floor((bounds.height - lineHeight) / 2), width: 2, height: lineHeight
    ).integral
  }

  private func updateCommandCaret() {
    guard let command = currentState.commandLine, currentState.isActive else {
      commandCaret.displayMode = .hidden
      commandCaret.isHidden = true
      return
    }
    commandCaret.frame = commandCaretFrame(command)
    commandCaret.color = EVThemeStore.shared.theme.statusBackground.color
    commandCaret.isHidden = false
    commandCaret.displayMode = .automatic
  }

  /// Whether the command caret is currently shown.
  public var isCommandCaretShowing: Bool { !commandCaret.isHidden }

  /// The command caret's frame in this view, for input-method anchoring.
  public func commandCaretRect() -> NSRect? {
    commandCaret.isHidden ? nil : commandCaret.frame
  }

  public override func mouseDragged(with event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    guard currentState.commandLine != nil, let offset = commandUTF8Offset(at: point) else {
      super.mouseDragged(with: event)
      return
    }
    commandLineDidSelect?(offset, true)
  }

  public override func layout() {
    super.layout()
    let area = commandAreaRect
    let inset = Self.contentInset
    let buttonWidth = min(20, max(0, area.width - inset))
    outputCloseButton.frame = NSRect(
      x: inset, y: 0, width: buttonWidth, height: bounds.height)
    let textStart = min(area.maxX, outputCloseButton.frame.maxX + 6)
    let font = commandFont
    let lineHeight = ceil(font.ascender - font.descender + font.leading)
    outputScroll.frame = NSRect(
      x: textStart, y: floor((bounds.height - lineHeight) / 2),
      width: max(0, area.maxX - textStart), height: lineHeight)
    outputTextView.minSize = outputScroll.contentSize
    outputTextView.sizeToFit()
    updateCommandCaret()
  }

  public func apply(_ state: EVStatusBarState) {
    currentState = state
    let showingOutput = state.commandLine == nil && state.commandOutput != nil
    leftGroup.isHidden = state.commandLine != nil || showingOutput
    outputScroll.isHidden = !showingOutput
    outputCloseButton.isHidden = !showingOutput
    if let output = state.commandOutput, outputTextView.string != output {
      outputTextView.string = output
      outputTextView.setSelectedRange(NSRange(location: 0, length: 0))
      outputTextView.scrollRangeToVisible(NSRange(location: 0, length: 0))
    }
    modeLabel.stringValue = state.mode
    messageLabel.stringValue = state.message
    locationLabel.title = state.location
    locationLabel.image = Self.lineIcon(state.lineMode)
    locationLabel.toolTip =
      (state.lineMode == .visual
        ? "Visual lines · click for physical source lines"
        : "Physical source lines · click for visual lines")
    if state.locationIsFragment {
      locationLabel.toolTip = "Position shows hard line · visual row. Click to change line mode."
    }
    let isMarkdown = [EVSourceFormat.markdown.displayName, EVSourceFormat.markdownSource.displayName].contains(state.format)
    formatSelect.isHidden = !isMarkdown
    formatLabel.isHidden = isMarkdown
    formatLabel.stringValue = state.format
    formatLabel.setAccessibilityLabel("Format: \(state.format)")
    formatSelect.selectItem(withTitle: state.format)
    formatSelect.invalidateIntrinsicContentSize()

    modeLabel.setAccessibilityLabel("Mode: \(state.mode)")
    locationLabel.setAccessibilityLabel(
      "\(state.lineMode == .visual ? "Visual" : "Physical source") lines: \(state.location)")
    formatSelect.setAccessibilityLabel("Format: \(state.format)")
    if let command = state.commandLine {
      setAccessibilityLabel("Command line: \(command.displayText)")
    } else if let output = state.commandOutput {
      setAccessibilityLabel("Command output: \(output)")
    } else {
      setAccessibilityLabel("Editor status")
    }
    applyTheme()
    needsLayout = true
    updateCommandCaret()
  }

  static func lineIcon(_ mode: EVLineMode) -> NSImage? {
    let path =
      mode == .visual
      ? "<path d='M1.5 8C4.5 2.5 11.5 2.5 14.5 8C11.5 13.5 4.5 13.5 1.5 8Z'/><circle cx='8' cy='8' r='2.1'/>"
      : "<path d='M4 1.5H9.5L12.5 4.5V14.5H4Z M9.5 1.5V4.5H12.5 M6 8H10 M6 10.5H10'/>"
    let svg =
      "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><g fill='none' stroke='black' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'>\(path)</g></svg>"
    let image = NSImage(data: Data(svg.utf8))
    image?.isTemplate = true
    return image
  }
}

/// Selection and copying stay native. All ordinary typing returns to the
/// editor without making the message editable or consuming the first key.
@MainActor
final class EVStatusOutputTextView: NSTextView {
  var didReceiveEditorKey: ((NSEvent) -> Void)?

  override func keyDown(with event: NSEvent) {
    let modifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
    let commandSelection = modifiers == .command
      && ["a", "c"].contains(event.charactersIgnoringModifiers?.lowercased() ?? "")
    let navigation = [115, 116, 119, 121, 123, 124, 125, 126].contains(Int(event.keyCode))
    if commandSelection || navigation {
      super.keyDown(with: event)
    } else {
      didReceiveEditorKey?(event)
    }
  }

  override func menu(for event: NSEvent) -> NSMenu? {
    let menu = NSMenu()
    let copy = menu.addItem(withTitle: "Copy", action: #selector(copy(_:)), keyEquivalent: "")
    copy.target = self
    let selectAll = menu.addItem(
      withTitle: "Select All", action: #selector(selectAll(_:)), keyEquivalent: "")
    selectAll.target = self
    return menu
  }
}

/// Native select tracking and keyboard accessibility with unobtrusive status chrome.
@MainActor
private final class EVStatusSelect: NSPopUpButton {
  var didChoose: ((EVStatusBarOption) -> Void)?
  private var options: [EVStatusBarOption] = []
  private var hoverTracking: NSTrackingArea?
  private var themeForeground = NSColor.labelColor

  init() {
    super.init(frame: .zero, pullsDown: false)
    cell = EVStatusSelectCell(textCell: "", pullsDown: false)
    isBordered = false
    font = .systemFont(ofSize: 11)
    controlSize = .small
    (cell as? NSPopUpButtonCell)?.arrowPosition = .noArrow
    setContentHuggingPriority(.required, for: .horizontal)
    setContentCompressionResistancePriority(.required, for: .horizontal)
    wantsLayer = true
    layer?.cornerRadius = 4
    target = self
    action = #selector(choose(_:))
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  func applyTheme(_ theme: EVTheme) {
    font = theme.statusFont
    themeForeground = theme.statusForeground.color
    (cell as? EVStatusSelectCell)?.themeForeground = themeForeground
    invalidateIntrinsicContentSize()
    needsDisplay = true
  }

  override var intrinsicContentSize: NSSize {
    let textWidth = (title as NSString).size(withAttributes: [
      .font: font ?? NSFont.systemFont(ofSize: 11)
    ]).width
    return NSSize(width: ceil(textWidth) + 26, height: super.intrinsicContentSize.height)
  }

  func configure(label: String, options: [(String, EVStatusBarOption)]) {
    self.options = options.map(\.1)
    addItems(withTitles: options.map(\.0))
    setAccessibilityLabel(label)
  }

  override func updateTrackingAreas() {
    if let hoverTracking { removeTrackingArea(hoverTracking) }
    let tracking = NSTrackingArea(
      rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
      owner: self, userInfo: nil
    )
    addTrackingArea(tracking)
    hoverTracking = tracking
    super.updateTrackingAreas()
  }

  override func mouseEntered(with event: NSEvent) {
    if isEnabled { layer?.backgroundColor = themeForeground.withAlphaComponent(0.12).cgColor }
  }

  override func mouseExited(with event: NSEvent) { layer?.backgroundColor = nil }

  @objc private func choose(_ sender: Any?) {
    guard options.indices.contains(indexOfSelectedItem) else { return }
    didChoose?(options[indexOfSelectedItem])
  }
}

@MainActor
private final class EVStatusSelectCell: NSPopUpButtonCell {
  var themeForeground = NSColor.labelColor
  override var cellSize: NSSize {
    var size = super.cellSize
    size.width += 10
    return size
  }

  override func draw(withFrame cellFrame: NSRect, in controlView: NSView) {
    var titleFrame = cellFrame
    titleFrame.size.width -= 10
    let textFont = font ?? NSFont.systemFont(ofSize: 11)
    let color = themeForeground.withAlphaComponent(isEnabled ? 1 : 0.4)
    let height = textFont.ascender - textFont.descender
    titleFrame.origin.y = cellFrame.midY - height / 2 - 1
    (title as NSString).draw(
      in: titleFrame, withAttributes: [.font: textFont, .foregroundColor: color])
    let x = cellFrame.maxX - 6
    let y = cellFrame.midY
    let triangle = NSBezierPath()
    triangle.move(to: NSPoint(x: x - 2.5, y: y - 1.5))
    triangle.line(to: NSPoint(x: x + 2.5, y: y - 1.5))
    triangle.line(to: NSPoint(x: x, y: y + 1.5))
    triangle.close()
    color.setFill()
    triangle.fill()
  }
}
