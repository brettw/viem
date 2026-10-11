import AppKit
import CoreText
import ViemCoreTextProvider

@MainActor
public final class EVStatusBarView: NSView, NSMenuItemValidation {
  public static var preferredHeight: CGFloat {
    let style = EVThemeStore.shared.chromeStyles.statusBar
    return ceil(style.fontHeight + style.padding.top + style.padding.bottom
      + style.topBorder.width + style.bottomBorder.width)
  }

  /// Shared status text inset comes from the style box.
  public static var contentInset: CGFloat {
    let style = EVThemeStore.shared.chromeStyles.statusBar
    return style.padding.left + style.leftBorder.width
  }
  private var barStyle: EVChromeStyle { EVThemeStore.shared.chromeStyles.statusBar }
  private var promptStyle: EVChromeStyle { EVThemeStore.shared.chromeStyles.statusCommand }
  private var barTextAttributes: [NSAttributedString.Key: Any] {
    var attributes: [NSAttributedString.Key: Any] = [.font: barStyle.font, .foregroundColor: barStyle.foreground]
    attributes.merge(letterSpacingAttributes(barStyle.letterSpacing)) { _, new in new }
    return attributes
  }
  private func applyLabelAttributes() {
    for label in [modeLabel, filePathLabel, messageLabel] {
      var attributes = barTextAttributes
      if label === messageLabel { attributes[.foregroundColor] = barStyle.foreground.withAlphaComponent(0.75) }
      label.attributedStringValue = NSAttributedString(string: label.stringValue, attributes: attributes)
    }
  }
  private var contentBox: NSRect {
    NSRect(x: barStyle.leftBorder.width, y: barStyle.bottomBorder.width,
      width: max(0, bounds.width - barStyle.leftBorder.width - barStyle.rightBorder.width),
      height: max(0, bounds.height - barStyle.topBorder.width - barStyle.bottomBorder.width))
  }
  private var leftInsetConstraint: NSLayoutConstraint!
  private var rightInsetConstraint: NSLayoutConstraint!
  private var leftCenterConstraint: NSLayoutConstraint!
  private var rightCenterConstraint: NSLayoutConstraint!

  /// Space between the command area and the caret position widget.
  static let commandAreaGap: CGFloat = 5

  var clicked: (() -> Void)?
  private var dragCursorPushed = false
  private lazy var clickGesture: EVStatusClickRecognizer = {
    let gesture = EVStatusClickRecognizer(target: self, action: #selector(observedClick))
    gesture.isEnabled = true
    gesture.clicked = { [weak self] in self?.clicked?() }
    return gesture
  }()
  public var preferredHeightDidChange: (() -> Void)?
  /// Top-to-bottom window delta; nil for the fixed bottom status bar.
  var dragDidMove: ((CGFloat) -> Void)? {
    didSet {
      resizeGesture.isEnabled = dragDidMove != nil
      if (oldValue == nil) != (dragDidMove == nil) { window?.invalidateCursorRects(for: self) }
    }
  }
  private lazy var resizeGesture: NSPanGestureRecognizer = {
    let gesture = NSPanGestureRecognizer(target: self, action: #selector(resizePanes(_:)))
    gesture.buttonMask = 1
    // A failed pan delivers the ordinary click to the original control,
    // including the eye button. A recognized pan consumes that click.
    gesture.delaysPrimaryMouseButtonEvents = true
    gesture.isEnabled = false
    return gesture
  }()
  private var lastDragTranslation: CGFloat = 0
  private var themeObserver: NSObjectProtocol?
  private var heightConstraint: NSLayoutConstraint!
  private var currentState = EVStatusBarState()
  private struct CommandShape {
    let prompt: String
    let text: String
    let font: NSFont
    let letterSpacing: CGFloat
    let line: CTLine
    let utf16Positions: [Int: Int]
    let caretStops: [(utf8: Int, x: CGFloat)]
  }
  /// Only the current command is retained. Moving its cursor or selecting text
  /// reuses the same full-line shaping, including ligatures and bidi context.
  private var commandShape: CommandShape?

  private let modeLabel = NSTextField(labelWithString: "")
  private let messageLabel = NSTextField(labelWithString: "")
  private let filePathLabel = NSTextField(labelWithString: "Untitled")
  private var modeWidthConstraint: NSLayoutConstraint!
  public var fileURL: URL? {
    didSet { filePath = EVStatusFilePath.display(fileURL) }
  }
  var writeCopiedPath: (String) -> Void = { path in
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(path, forType: .string)
  }
  public private(set) var filePath: String = "Untitled" {
    didSet {
      guard filePath != oldValue else { return }
      filePathLabel.stringValue = filePath
      applyLabelAttributes()
      filePathLabel.toolTip = filePath
      filePathLabel.setAccessibilityLabel("File: \(filePath)")
    }
  }
  private let locationLabel = NSButton(title: "", target: nil, action: nil)
  public var optionDidChange: ((EVStatusBarOption) -> Void)?
  /// A click inside the command area, as a UTF-8 offset into its text.
  public var commandLineDidSelect: ((Int, Bool) -> Void)?
  public var commandOutputDidDismiss: (() -> Void)?
  public var commandOutputDidReceiveKey: ((NSEvent) -> Void)?
  private let leftGroup = NSStackView()
  private var leftGroupTrailingConstraint: NSLayoutConstraint!
  private let commandCaret = NSTextInsertionIndicator(frame: .zero)
  @MainActor
  private struct OutputControls {
    let scroll = NSScrollView()
    let text = EVStatusOutputTextView()
    let close = NSButton()
  }
  private var outputControls: OutputControls?
  var outputTextView: EVStatusOutputTextView? { outputControls?.text }

  public override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    translatesAutoresizingMaskIntoConstraints = false
    setAccessibilityRole(.group)
    setAccessibilityLabel("Editor status")
    addGestureRecognizer(resizeGesture)
    addGestureRecognizer(clickGesture)

    wantsLayer = true
    layer?.masksToBounds = true
    locationLabel.isBordered = false
    locationLabel.imagePosition = .imageLeading
    locationLabel.imageScaling = .scaleProportionallyDown
    locationLabel.target = self
    locationLabel.action = #selector(toggleLineMode)
    locationLabel.setContentHuggingPriority(.required, for: .horizontal)
    modeLabel.font = .systemFont(ofSize: 11, weight: .semibold)
    modeLabel.setContentHuggingPriority(.defaultHigh, for: .horizontal)
    modeWidthConstraint = modeLabel.widthAnchor.constraint(equalToConstant: 0)
    modeWidthConstraint.isActive = true
    filePathLabel.lineBreakMode = .byTruncatingHead
    filePathLabel.maximumNumberOfLines = 1
    filePathLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    filePathLabel.setAccessibilityLabel("File: Untitled")
    let pathMenu = NSMenu()
    for (title, action) in [("Copy full path", #selector(copyFullPath(_:))),
                            ("Copy relative path", #selector(copyRelativePath(_:)))] {
      let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
      item.target = self
      pathMenu.addItem(item)
    }
    filePathLabel.menu = pathMenu
    messageLabel.textColor = .secondaryLabelColor
    messageLabel.lineBreakMode = .byTruncatingTail
    messageLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)

    // Everything except the caret position widget is left-aligned; the widget
    // is always present and always at the trailing edge.
    leftGroup.setViews([modeLabel, filePathLabel, messageLabel], in: .leading)
    leftGroup.orientation = .horizontal
    leftGroup.spacing = 14
    leftGroup.alignment = .centerY
    leftGroup.translatesAutoresizingMaskIntoConstraints = false
    locationLabel.translatesAutoresizingMaskIntoConstraints = false
    commandCaret.displayMode = .hidden
    commandCaret.isHidden = true
    commandCaret.automaticModeOptions = [.showEffectsView, .showWhileTracking]

    addSubview(leftGroup)
    addSubview(locationLabel)
    addSubview(commandCaret)
    heightConstraint = heightAnchor.constraint(equalToConstant: Self.preferredHeight)
    leftGroupTrailingConstraint = leftGroup.trailingAnchor.constraint(
      lessThanOrEqualTo: locationLabel.leadingAnchor, constant: -14)
    let inset = Self.contentInset
    leftInsetConstraint = leftGroup.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset)
    rightInsetConstraint = locationLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset)
    leftCenterConstraint = leftGroup.centerYAnchor.constraint(equalTo: centerYAnchor)
    rightCenterConstraint = locationLabel.centerYAnchor.constraint(equalTo: centerYAnchor)
    NSLayoutConstraint.activate([heightConstraint, leftInsetConstraint, leftCenterConstraint,
      leftGroupTrailingConstraint, rightInsetConstraint, rightCenterConstraint])

    themeObserver = NotificationCenter.default.addObserver(
      forName: .viemThemeDidChange, object: EVThemeStore.shared, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated {
        self?.applyTheme()
        self?.preferredHeightDidChange?()
      }
    }
    applyTheme()
    apply(EVStatusBarState())
  }

  deinit {
    if dragCursorPushed { NSCursor.pop() }
    if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
  }

  /// Ordinary editing needs no TextKit document. Create the read-only output
  /// surface on its first message, retaining it for subsequent selections.
  private func ensureOutputControls() {
    guard outputControls == nil else { return }
    let controls = OutputControls()
    controls.text.isEditable = false
    controls.text.isSelectable = true
    controls.text.isRichText = false
    controls.text.drawsBackground = false
    controls.text.textContainerInset = .zero
    controls.text.textContainer?.lineFragmentPadding = 0
    controls.text.textContainer?.widthTracksTextView = false
    controls.text.textContainer?.containerSize = NSSize(
      width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
    controls.text.isHorizontallyResizable = true
    controls.text.isVerticallyResizable = true
    controls.text.maxSize = NSSize(
      width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
    controls.text.didReceiveEditorKey = { [weak self] event in
      self?.commandOutputDidReceiveKey?(event)
    }
    controls.text.setAccessibilityLabel("Command output")
    controls.scroll.documentView = controls.text
    controls.scroll.drawsBackground = false
    controls.scroll.borderType = .noBorder
    controls.scroll.hasHorizontalScroller = false
    controls.scroll.hasVerticalScroller = false
    addSubview(controls.scroll)
    controls.close.image = NSImage(
      systemSymbolName: "xmark", accessibilityDescription: "Close command output")
    controls.close.imagePosition = .imageOnly
    controls.close.isBordered = false
    controls.close.target = self
    controls.close.action = #selector(dismissCommandOutput(_:))
    controls.close.setAccessibilityLabel("Close command output")
    addSubview(controls.close)
    outputControls = controls
    applyOutputTheme()
  }

  private func applyOutputTheme() {
    guard let controls = outputControls else { return }
    controls.text.font = barStyle.font
    controls.text.typingAttributes = [.font: barStyle.font, .foregroundColor: barStyle.foreground]
      .merging(letterSpacingAttributes(barStyle.letterSpacing)) { _, new in new }
    controls.text.textColor = barStyle.foreground
    controls.text.insertionPointColor = .clear
    controls.close.contentTintColor = barStyle.foreground
  }

  private func updateLocationTitle() {
    locationLabel.attributedTitle = NSAttributedString(
      string: currentState.location,
      attributes: barTextAttributes)
    locationLabel.invalidateIntrinsicContentSize()
  }

  private func applyTheme() {
    let style = barStyle
    let font = style.font
    layer?.backgroundColor = style.background?.cgColor
    modeLabel.font = font
    messageLabel.font = font
    filePathLabel.font = font
    filePathLabel.textColor = style.foreground
    modeWidthConstraint.constant = ceil([
      "NORMAL", "INSERT", "REPLACE", "VISUAL", "VISUAL LINE", "VISUAL BLOCK",
      "SELECTION", "SELECT", "SELECT LINE", "SELECT BLOCK", "COMMAND",
    ].map { ($0 as NSString).size(withAttributes: barTextAttributes).width }.max() ?? 0) + 4
    modeLabel.textColor = style.foreground
    messageLabel.textColor = style.foreground.withAlphaComponent(0.75)
    locationLabel.font = font
    locationLabel.contentTintColor = style.foreground
    applyLabelAttributes()
    updateLocationTitle()
    applyOutputTheme()
    heightConstraint.constant = Self.preferredHeight
    leftInsetConstraint.constant = style.leftBorder.width + style.padding.left
    rightInsetConstraint.constant = -(style.rightBorder.width + style.padding.right)
    let center = (style.bottomBorder.width + style.padding.bottom - style.topBorder.width - style.padding.top) / 2
    leftCenterConstraint.constant = center
    rightCenterConstraint.constant = center
    needsLayout = true
    needsDisplay = true
    updateCommandCaret()
  }

  public func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
    fileURL != nil
  }

  @objc private func copyFullPath(_ sender: NSMenuItem) {
    guard let fileURL else { return }
    writeCopiedPath(fileURL.standardizedFileURL.path)
  }

  @objc private func copyRelativePath(_ sender: NSMenuItem) {
    guard let fileURL else { return }
    writeCopiedPath(EVStatusFilePath.relative(fileURL))
  }

  @objc private func observedClick() {}

  @objc private func toggleLineMode() {
    optionDidChange?(.lineMode(currentState.lineMode == .visual ? .physicalSource : .visual))
  }

  @objc private func resizePanes(_ gesture: NSPanGestureRecognizer) {
    switch gesture.state {
    case .began, .changed:
      if gesture.state == .began { lastDragTranslation = 0; NSCursor.resizeUpDown.push(); dragCursorPushed = true }
      let translation = -gesture.translation(in: nil).y
      let delta = translation - lastDragTranslation
      lastDragTranslation = translation // Discard blocked overshoot at an edge.
      dragDidMove?(delta)
    default:
      lastDragTranslation = 0
      if dragCursorPushed { NSCursor.pop(); dragCursorPushed = false }
    }
  }

  @objc private func dismissCommandOutput(_ sender: Any?) { commandOutputDidDismiss?() }

  public var isCommandOutputFocused: Bool {
    guard let controls = outputControls else { return false }
    return !controls.scroll.isHidden && window?.firstResponder === controls.text
  }

  /// Native menu commands must copy the selected message, even though the
  /// application menu normally routes editor commands through the core.
  public func commandOutputPresentation(for command: EVMenuCommand) -> EVMenuItemPresentation? {
    guard isCommandOutputFocused, let text = outputControls?.text else { return nil }
    switch command {
    case .copy, .copySource:
      return EVMenuItemPresentation(isEnabled: text.selectedRange().length > 0)
    case .selectAll: return .enabled
    case .cut, .delete: return .disabled
    default: return nil
    }
  }

  public func performCommandOutputAction(_ command: EVMenuCommand) -> Bool {
    guard isCommandOutputFocused, let text = outputControls?.text else { return false }
    switch command {
    case .copy, .copySource: text.copy(nil)
    case .selectAll: text.selectAll(nil)
    case .cut, .delete: break // Output is read-only, including menu actions.
    default: return false
    }
    return true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  private var commandFont: NSFont { promptStyle.font }
  private var commandTextAttributes: [NSAttributedString.Key: Any] {
    var attributes: [NSAttributedString.Key: Any] = [.font: commandFont, .foregroundColor: promptStyle.foreground]
    attributes.merge(letterSpacingAttributes(promptStyle.letterSpacing)) { _, new in new }
    return attributes
  }

  /// The command area runs from the leading window edge to the caret position
  /// widget, less the gap. Its background reaches the edge; its text does not.
  var commandAreaRect: NSRect {
    let box = contentBox
    let limit = min(box.maxX, max(box.minX, locationLabel.frame.minX - Self.commandAreaGap))
    return NSRect(x: box.minX, y: box.minY, width: limit - box.minX, height: box.height)
  }

  private func shapeCommand(_ command: EVStatusCommandLine) -> CommandShape {
    let font = commandFont
    let spacing = promptStyle.letterSpacing
    if let shape = commandShape, shape.prompt == command.prompt, shape.text == command.text,
      shape.font.isEqual(font), shape.letterSpacing == spacing
    {
      return shape
    }
    let line = CTLineCreateWithAttributedString(NSAttributedString(
      string: command.displayText, attributes: commandTextAttributes))
    let promptLength = command.prompt.utf16.count
    var utf16Positions = [0: promptLength]
    var utf8Offset = 0
    var utf16Offset = promptLength
    // Composition can place the caret at a scalar boundary within a grapheme.
    // Keep those UTF-8 positions distinct from the pointer's grapheme stops.
    for scalar in command.text.unicodeScalars {
      utf8Offset += scalar.utf8.count
      utf16Offset += scalar.utf16.count
      utf16Positions[utf8Offset] = utf16Offset
    }
    var caretStops = [(utf8: 0, x: CTLineGetOffsetForStringIndex(line, promptLength, nil))]
    utf8Offset = 0
    utf16Offset = promptLength
    for character in command.text {
      let text = String(character)
      utf8Offset += text.utf8.count
      utf16Offset += text.utf16.count
      caretStops.append((utf8: utf8Offset, x: CTLineGetOffsetForStringIndex(line, utf16Offset, nil)))
    }
    let shape = CommandShape(prompt: command.prompt, text: command.text, font: font,
      letterSpacing: spacing, line: line, utf16Positions: utf16Positions, caretStops: caretStops)
    commandShape = shape
    return shape
  }

  /// Horizontal offset of one UTF-8 position within the fully shaped command,
  /// before scrolling. Measuring a prefix would lose following ligature context.
  private func commandTextX(upTo utf8Offset: Int, in command: EVStatusCommandLine) -> CGFloat {
    let shape = shapeCommand(command)
    let offset = min(max(0, utf8Offset), command.text.utf8.count)
    guard let index = shape.utf16Positions[offset] else { return 0 }
    return CTLineGetOffsetForStringIndex(shape.line, index, nil)
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
    for stop in shapeCommand(command).caretStops {
      let x = origin + stop.x
      if abs(x - point.x) < distance {
        distance = abs(x - point.x)
        best = stop.utf8
      }
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
    drawBorders(barStyle, in: bounds)
    guard let command = currentState.commandLine else { return }
    let background = promptStyle.background ?? .clear
    let foreground = promptStyle.foreground
    let area = commandAreaRect
    background.setFill()
    area.fill()

    let font = commandFont
    let text = command.displayText
    let rendered = NSMutableAttributedString(
      string: text, attributes: commandTextAttributes)
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
      y: floor(commandAreaRect.midY - lineHeight / 2))
    NSGraphicsContext.saveGraphicsState()
    NSBezierPath(rect: NSRect(
      x: Self.contentInset, y: area.minY,
      width: max(0, area.maxX - Self.contentInset), height: area.height)).setClip()
    rendered.draw(at: origin)
    // Preserve the thin command caret's geometry when outlining it steadily.
    if !currentState.isActive {
      let outline = commandCaretFrame(command)
      foreground.withAlphaComponent(0.75).setStroke()
      NSBezierPath(rect: outline.insetBy(dx: 0.5, dy: 0.5)).stroke()
    }
    NSGraphicsContext.restoreGraphicsState()
  }

  private func drawBorders(_ style: EVChromeStyle, in rect: NSRect) {
    for (border, edge) in [
      (style.topBorder, NSRect(x: rect.minX, y: rect.maxY - style.topBorder.width, width: rect.width, height: style.topBorder.width)),
      (style.rightBorder, NSRect(x: rect.maxX - style.rightBorder.width, y: rect.minY, width: style.rightBorder.width, height: rect.height)),
      (style.bottomBorder, NSRect(x: rect.minX, y: rect.minY, width: rect.width, height: style.bottomBorder.width)),
      (style.leftBorder, NSRect(x: rect.minX, y: rect.minY, width: style.leftBorder.width, height: rect.height)),
    ] where border.width > 0 {
      (border.color ?? style.foreground).setFill(); edge.fill()
    }
  }

  /// Caret rectangle for the command line, in this view's coordinates.
  private func commandCaretFrame(_ command: EVStatusCommandLine) -> NSRect {
    let font = commandFont
    let lineHeight = ceil(font.ascender - font.descender + font.leading)
    let x = Self.contentInset - commandScroll(command)
      + commandTextX(upTo: command.cursorUTF8Offset, in: command)
    let clipped = NSRect(
      x: floor(x), y: floor(commandAreaRect.midY - lineHeight / 2), width: 2, height: lineHeight
    ).intersection(commandAreaRect)
    return clipped.isNull ? .zero : clipped
  }

  private func updateCommandCaret() {
    guard let command = currentState.commandLine, currentState.isActive else {
      commandCaret.displayMode = .hidden
      commandCaret.isHidden = true
      return
    }
    let frame = commandCaretFrame(command)
    guard !frame.isEmpty else {
      commandCaret.displayMode = .hidden
      commandCaret.isHidden = true
      return
    }
    commandCaret.frame = frame
    commandCaret.color = promptStyle.foreground
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

  public override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

  /// Empty background remains a resize target; native controls and command
  /// selection retain their usual hover cursors.
  var resizeCursorRects: [NSRect] {
    guard dragDidMove != nil, !isHiddenOrHasHiddenAncestor else { return [] }
    var controls: [NSView] = [modeLabel, filePathLabel, messageLabel, locationLabel]
    if let outputControls { controls += [outputControls.close, outputControls.scroll] }
    var exclusions = controls.filter { !$0.isHiddenOrHasHiddenAncestor }.map { control in
      var rect = control.bounds
      if let label = control as? NSTextField {
        rect = label.cell?.titleRect(forBounds: rect) ?? rect
        let width = min(rect.width, label.attributedStringValue.size().width)
        if label.alignment == .right { rect.origin.x = rect.maxX - width }
        else if label.alignment == .center { rect.origin.x = rect.midX - width / 2 }
        rect.size.width = width
      }
      return control.convert(rect, to: self)
    }
    if currentState.commandLine != nil { exclusions.append(commandAreaRect) }
    var areas = [bounds]
    for excluded in exclusions {
      areas = areas.flatMap { area -> [NSRect] in
        let overlap = area.intersection(excluded)
        guard !overlap.isEmpty else { return [area] }
        return [
          NSRect(x: area.minX, y: area.minY, width: area.width, height: overlap.minY - area.minY),
          NSRect(x: area.minX, y: overlap.maxY, width: area.width, height: area.maxY - overlap.maxY),
          NSRect(x: area.minX, y: overlap.minY, width: overlap.minX - area.minX, height: overlap.height),
          NSRect(x: overlap.maxX, y: overlap.minY, width: area.maxX - overlap.maxX, height: overlap.height),
        ].filter { !$0.isEmpty }
      }
    }
    return areas
  }

  public override func resetCursorRects() {
    super.resetCursorRects()
    for rect in resizeCursorRects { addCursorRect(rect, cursor: .resizeUpDown) }
  }

  public override func layout() {
    let hideLabels = currentState.commandLine != nil || currentState.commandOutput != nil || bounds.width < 320
    setLeftGroupHidden(hideLabels)
    // Refresh native button metrics after attachment or a change in split
    // width, so a compressed position widget recovers when widened.
    locationLabel.invalidateIntrinsicContentSize()
    super.layout()
    if let controls = outputControls, !controls.scroll.isHidden {
      let area = commandAreaRect
      let inset = Self.contentInset
      let buttonWidth = min(20, max(0, area.width - inset))
      controls.close.frame = NSRect(
        x: inset, y: area.minY, width: buttonWidth, height: area.height)
      let textStart = min(area.maxX, controls.close.frame.maxX + 6)
      let lineHeight = barStyle.fontHeight
      let outputFrame = NSRect(
        x: textStart, y: floor(area.midY - lineHeight / 2),
        width: max(0, area.maxX - textStart), height: lineHeight).intersection(area)
      controls.scroll.frame = outputFrame.isNull ? .zero : outputFrame
      controls.text.minSize = controls.scroll.contentSize
      controls.text.sizeToFit()
    }
    updateCommandCaret()
    window?.invalidateCursorRects(for: self)
  }

  public func apply(_ state: EVStatusBarState) {
    let previous = currentState
    currentState = state
    if state.commandLine == nil { commandShape = nil }
    let showingOutput = state.commandLine == nil && state.commandOutput != nil
    setLeftGroupHidden(state.commandLine != nil || showingOutput || bounds.width < 320)
    if state.commandOutput != nil { ensureOutputControls() }
    if let controls = outputControls {
      controls.scroll.isHidden = !showingOutput
      controls.close.isHidden = !showingOutput
      if let output = state.commandOutput, controls.text.string != output {
        controls.text.string = output
        controls.text.setSelectedRange(NSRange(location: 0, length: 0))
        controls.text.scrollRangeToVisible(NSRange(location: 0, length: 0))
      }
    }
    modeLabel.stringValue = state.mode
    messageLabel.stringValue = state.message
    applyLabelAttributes()
    messageLabel.isHidden = state.message.isEmpty
    if previous.location != state.location { updateLocationTitle() }
    if previous.lineMode != state.lineMode || locationLabel.image == nil {
      locationLabel.image = Self.lineIcon(state.lineMode)
    }
    locationLabel.toolTip =
      (state.lineMode == .visual
        ? "Visual lines · click for physical source lines"
        : "Physical source lines · click for visual lines")
    if state.locationIsFragment {
      locationLabel.toolTip = "Position shows hard line · visual row. Click to change line mode."
    }
    modeLabel.setAccessibilityLabel("Mode: \(state.mode)")
    locationLabel.setAccessibilityLabel(
      "\(state.lineMode == .visual ? "Visual" : "Physical source") lines: \(state.location)")
    if let command = state.commandLine {
      setAccessibilityLabel("Command line: \(command.displayText)")
    } else if let output = state.commandOutput {
      setAccessibilityLabel("Command output: \(output)")
    } else {
      setAccessibilityLabel("Editor status")
    }
    needsLayout = true
    needsDisplay = true
    updateCommandCaret()
  }

  private func setLeftGroupHidden(_ hidden: Bool) {
    if leftGroup.isHidden != hidden { leftGroup.isHidden = hidden }
    // An invisible label group must not squeeze the position widget. Its
    // intrinsic width remains stable as a pane narrows and widens again.
    leftGroupTrailingConstraint.isActive = !hidden
  }

  static func lineIcon(_ mode: EVLineMode) -> NSImage? {
    let path =
      mode == .visual
      ? "<path d='M1.5 8C4.5 2.5 11.5 2.5 14.5 8C11.5 13.5 4.5 13.5 1.5 8Z'/><circle cx='8' cy='8' r='2.1'/>"
      : "<path d='M4 1.5H9.5L12.5 4.5V14.5H4Z M9.5 1.5V4.5H12.5 M6 8H10 M6 10.5H10'/>"
    let svg =
      "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><g fill='none' stroke='black' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'>\(path)</g></svg>"
    let image = NSImage(data: Data(svg.utf8))
    image?.size = NSSize(width: 16, height: 16)
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

/// Observe ordinary clicks without recognizing (and consuming) a gesture. The
/// pan recognizer still delays child events and consumes recognized drags.
@MainActor
private final class EVStatusClickRecognizer: NSClickGestureRecognizer {
  var clicked: (() -> Void)?
  private var press: NSPoint?
  private var moved = false
  override func mouseDown(with event: NSEvent) { super.mouseDown(with: event); press = event.locationInWindow; moved = false }
  override func mouseDragged(with event: NSEvent) {
    super.mouseDragged(with: event)
    if let press, hypot(event.locationInWindow.x - press.x, event.locationInWindow.y - press.y) >= 4 { moved = true }
  }
  override func mouseUp(with event: NSEvent) {
    if press != nil && !moved { clicked?() }
    state = .failed
  }
  override func reset() { press = nil; moved = false; super.reset() }
}
