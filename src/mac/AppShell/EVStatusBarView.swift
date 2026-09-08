import AppKit

@MainActor
public final class EVStatusBarView: NSView {
  public static var preferredHeight: CGFloat {
    max(25, ceil(EVThemeStore.shared.theme.statusFontSize + 14))
  }
  public var preferredHeightDidChange: (() -> Void)?
  private var themeObserver: NSObjectProtocol?
  private var heightConstraint: NSLayoutConstraint!
  private var currentState = EVStatusBarState()

  private let modeLabel = NSTextField(labelWithString: "")
  private let messageLabel = NSTextField(labelWithString: "")
  private let locationLabel = NSButton(title: "", target: nil, action: nil)
  public var optionDidChange: ((EVStatusBarOption) -> Void)?
  private let formatSelect = EVStatusSelect()

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
      options: [EVSourceFormat.plainText, .markdownSource, .markdown, .htmlSource, .html, .rtf].map {
        ($0.displayName, .format($0))
      }
    )
    formatSelect.didChoose = { [weak self] option in self?.optionDidChange?(option) }
    let trailing = NSStackView(views: [
      locationLabel, formatSelect,
    ])
    trailing.orientation = .horizontal
    trailing.spacing = 14
    trailing.alignment = .centerY
    trailing.setContentHuggingPriority(.required, for: .horizontal)

    let row = NSStackView(views: [modeLabel, messageLabel, trailing])
    row.orientation = .horizontal
    row.spacing = 14
    row.alignment = .centerY
    row.translatesAutoresizingMaskIntoConstraints = false

    addSubview(separator)
    addSubview(row)
    heightConstraint = heightAnchor.constraint(equalToConstant: Self.preferredHeight)
    NSLayoutConstraint.activate([
      heightConstraint,
      separator.leadingAnchor.constraint(equalTo: leadingAnchor),
      separator.trailingAnchor.constraint(equalTo: trailingAnchor),
      separator.topAnchor.constraint(equalTo: topAnchor),
      row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
      row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
      row.topAnchor.constraint(equalTo: separator.bottomAnchor),
      row.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])

    themeObserver = NotificationCenter.default.addObserver(
      forName: .evimThemeDidChange, object: EVThemeStore.shared, queue: .main
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
    heightConstraint.constant = Self.preferredHeight
    needsDisplay = true
  }

  @objc private func toggleLineMode() {
    optionDidChange?(.lineMode(currentState.lineMode == .visual ? .physicalSource : .visual))
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  public func apply(_ state: EVStatusBarState) {
    currentState = state
    modeLabel.stringValue = state.mode
    messageLabel.stringValue = state.message
    locationLabel.title = state.location
    locationLabel.image = Self.lineIcon(state.lineMode)
    locationLabel.isEnabled = state.format != EVSourceFormat.rtf.displayName
    locationLabel.toolTip =
      state.format == EVSourceFormat.rtf.displayName
      ? "Visual lines · RTF has no physical source lines"
      : (state.lineMode == .visual
        ? "Visual lines · click for physical source lines"
        : "Physical source lines · click for visual lines")
    if state.locationIsFragment {
      locationLabel.toolTip = "Position shows hard line · visual row. Click to change line mode."
    }
    formatSelect.selectItem(withTitle: state.format)
    formatSelect.invalidateIntrinsicContentSize()

    modeLabel.setAccessibilityLabel("Mode: \(state.mode)")
    locationLabel.setAccessibilityLabel(
      "\(state.lineMode == .visual ? "Visual" : "Physical source") lines: \(state.location)")
    formatSelect.setAccessibilityLabel("Format: \(state.format)")
    applyTheme()
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
