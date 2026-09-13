import AppKit

/// Keeps the native well's drag/drop and color-panel behavior, while supplying
/// a compact palette whose current selection is visible, including custom colors.
@MainActor
final class EVStyleColorWell: NSColorWell, NSPopoverDelegate {
    var includesTransparent = false
    private static weak var panelOwner: EVStyleColorWell?
    private(set) var palettePopover: NSPopover?
    private(set) var paletteController: EVStyleColorPaletteController?
    private(set) var displayedColor = EVStyleColor(red: 0, green: 0, blue: 0, alpha: 1)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configurePulldown()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configurePulldown()
    }

    private func configurePulldown() {
        colorWellStyle = .minimal
        supportsAlpha = true
        pulldownTarget = self
        pulldownAction = #selector(showPalette(_:))
    }

    override var color: NSColor {
        didSet {
            guard let rgb = color.usingColorSpace(.deviceRGB) else { return }
            displayedColor = EVStyleColor(red: Float(rgb.redComponent), green: Float(rgb.greenComponent),
                blue: Float(rgb.blueComponent), alpha: Float(rgb.alphaComponent))
            refreshPalette()
        }
    }

    /// Source-normalized components are authoritative for selection and its
    /// text; color-space conversion must not turn an exact preset into a custom color.
    func setCommittedColor(_ value: EVStyleColor, displayColor: NSColor? = nil) {
        color = displayColor ?? value.appKitColor
        displayedColor = value
        refreshPalette()
    }

    private func refreshPalette() {
        guard let controller = paletteController else { return }
        controller.update(current: displayedColor)
        palettePopover?.contentSize = controller.preferredContentSize
    }

    override var isEnabled: Bool {
        didSet { if !isEnabled { dismissColorControls() } }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { dismissColorControls() }
    }

    @objc func showPalette(_ sender: Any?) {
        guard isEnabled, window != nil else { return }
        if palettePopover?.isShown == true { palettePopover?.close(); return }
        let controller = EVStyleColorPaletteController(current: displayedColor, includesTransparent: includesTransparent)
        controller.onChoose = { [weak self] in self?.choose($0) }
        controller.onMoreColors = { [weak self] in self?.showMoreColors() }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        popover.contentViewController = controller
        controller.loadViewIfNeeded()
        popover.contentSize = controller.preferredContentSize
        paletteController = controller
        palettePopover = popover
        popover.show(relativeTo: bounds, of: self, preferredEdge: .maxY)
    }

    private func choose(_ value: EVStyleColor) {
        guard isEnabled else { return }
        if value != displayedColor {
            color = NSColor(deviceRed: CGFloat(value.red), green: CGFloat(value.green),
                blue: CGFloat(value.blue), alpha: CGFloat(value.alpha))
            _ = sendAction(action, to: target)
        }
        palettePopover?.close()
    }

    func showMoreColors() {
        guard isEnabled else { return }
        palettePopover?.close()
        let panel = NSColorPanel.shared
        // Direct typography also uses this shared panel. An active style well
        // owns the interaction; it must not call a previously installed target.
        panel.setTarget(nil)
        panel.setAction(nil)
        activate(true)
        Self.panelOwner = self
        panel.makeKeyAndOrderFront(nil)
    }

    static func deactivatePanelOwner() {
        panelOwner?.deactivate()
        panelOwner = nil
    }

    func dismissColorControls() {
        palettePopover?.close()
        palettePopover = nil
        paletteController = nil
        if isActive { deactivate() }
        if Self.panelOwner === self { Self.panelOwner = nil }
    }

    func popoverDidClose(_ notification: Notification) {
        guard notification.object as? NSPopover === palettePopover else { return }
        palettePopover = nil
        paletteController = nil
    }
}

@MainActor
final class EVStyleColorPaletteController: NSViewController {
    struct Choice {
        let name: String
        let color: EVStyleColor
    }

    private static let namedColors: [(String, UInt32)] = [
        ("Black", 0x000000), ("Gray", 0x808080), ("Silver", 0xC0C0C0), ("White", 0xFFFFFF), ("Slate", 0x34495E), ("Brown", 0x8B4513),
        ("Red", 0xFF0000), ("Orange", 0xFF8000), ("Yellow", 0xFFFF00), ("Green", 0x008000), ("Teal", 0x008080), ("Blue", 0x0000FF),
        ("Pink", 0xFF80C0), ("Peach", 0xFFC080), ("Cream", 0xFFFF80), ("Light green", 0x80FF80), ("Cyan", 0x00FFFF), ("Lilac", 0xC080FF),
        ("Maroon", 0x800000), ("Rust", 0x804000), ("Olive", 0x808000), ("Forest green", 0x004000), ("Navy", 0x000080), ("Purple", 0x800080),
    ]
    static let choices: [Choice] = namedColors.map { name, rgb in
        let red = Float((rgb >> 16) & 255) / 255
        let green = Float((rgb >> 8) & 255) / 255
        let blue = Float(rgb & 255) / 255
        return Choice(name: name, color: EVStyleColor(red: red, green: green, blue: blue, alpha: 1))
    }
    static let transparent = EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0)

    var onChoose: ((EVStyleColor) -> Void)?
    var onMoreColors: (() -> Void)?
    private(set) var currentColor: EVStyleColor
    private(set) var choiceButtons: [EVStyleColorSwatchButton] = []
    private(set) var currentSwatch = EVStyleColorSwatchButton()
    private(set) var currentValueLabel = NSTextField(wrappingLabelWithString: "")
    private(set) var transparentButton: NSButton?
    private let includesTransparent: Bool

    init(current: EVStyleColor, includesTransparent: Bool) {
        currentColor = current
        self.includesTransparent = includesTransparent
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        let root = NSView()
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            root.widthAnchor.constraint(equalToConstant: 258),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -12),
        ])
        currentSwatch.setAccessibilityLabel("Current color")
        currentSwatch.target = self
        currentSwatch.action = #selector(chooseCurrent(_:))
        let currentTitle = NSTextField(labelWithString: "Current")
        currentTitle.font = .systemFont(ofSize: 12, weight: .medium)
        currentValueLabel.font = .monospacedSystemFont(ofSize: 10, weight: .regular)
        currentValueLabel.textColor = .secondaryLabelColor
        currentValueLabel.isSelectable = true
        currentValueLabel.preferredMaxLayoutWidth = 190
        currentValueLabel.widthAnchor.constraint(equalToConstant: 190).isActive = true
        currentValueLabel.setAccessibilityLabel("Current color value")
        let labels = NSStackView(views: [currentTitle, currentValueLabel])
        labels.orientation = .vertical
        labels.alignment = .leading
        labels.spacing = 2
        let currentRow = NSStackView(views: [currentSwatch, labels])
        currentRow.spacing = 12
        currentRow.alignment = .centerY
        stack.addArrangedSubview(currentRow)
        let grid = NSStackView()
        grid.orientation = .vertical
        grid.alignment = .leading
        grid.spacing = 6
        for rowStart in stride(from: 0, to: Self.choices.count, by: 6) {
            let row = NSStackView()
            row.spacing = 6
            for index in rowStart..<(rowStart + 6) {
                let choice = Self.choices[index]
                let button = EVStyleColorSwatchButton()
                button.swatchColor = choice.color
                button.tag = index
                button.setAccessibilityLabel(choice.name)
                button.toolTip = "\(choice.name) · \(Self.description(of: choice.color))"
                button.target = self
                button.action = #selector(choosePreset(_:))
                choiceButtons.append(button)
                row.addArrangedSubview(button)
            }
            grid.addArrangedSubview(row)
        }
        stack.addArrangedSubview(grid)
        if includesTransparent {
            let button = NSButton(checkboxWithTitle: "Transparent", target: self, action: #selector(chooseTransparent(_:)))
            button.setButtonType(.pushOnPushOff)
            button.bezelStyle = .rounded
            button.setAccessibilityLabel("Transparent")
            transparentButton = button
            stack.addArrangedSubview(button)
        }
        let more = NSButton(title: "More Colors…", target: self, action: #selector(showMoreColors(_:)))
        more.bezelStyle = .rounded
        more.setAccessibilityLabel("More Colors")
        stack.addArrangedSubview(more)
        more.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        view = root
        update(current: currentColor)
    }

    func update(current: EVStyleColor) {
        currentColor = current
        guard isViewLoaded else { return }
        currentSwatch.swatchColor = current
        currentSwatch.state = .on
        currentSwatch.setAccessibilityValue(Self.description(of: current))
        currentSwatch.toolTip = Self.description(of: current)
        currentValueLabel.stringValue = Self.description(of: current)
        for (index, button) in choiceButtons.enumerated() {
            button.state = Self.choices[index].color == current ? .on : .off
        }
        transparentButton?.state = current == Self.transparent ? .on : .off
        preferredContentSize = NSSize(width: 258, height: view.fittingSize.height)
    }

    static func description(of value: EVStyleColor) -> String {
        let values = [value.red, value.green, value.blue, value.alpha]
        if values.allSatisfy({ $0 >= 0 && $0 <= 1 && ($0 * 255).rounded() / 255 == $0 }) {
            let components = values.map { Int(($0 * 255).rounded()) }
            return value.alpha == 1
                ? String(format: "#%02X%02X%02X", components[0], components[1], components[2])
                : String(format: "#%02X%02X%02X%02X", components[0], components[1], components[2], components[3])
        }
        return "RGBA \(values.map { String($0) }.joined(separator: ", "))"
    }

    @objc private func chooseCurrent(_ sender: NSButton) { onChoose?(currentColor) }
    @objc private func choosePreset(_ sender: NSButton) { onChoose?(Self.choices[sender.tag].color) }
    @objc private func chooseTransparent(_ sender: NSButton) { onChoose?(Self.transparent) }
    @objc private func showMoreColors(_ sender: NSButton) { onMoreColors?() }
}

/// A standard button supplies keyboard, accessibility, and tracking behavior;
/// only the swatch and contrast-safe selection mark are custom drawing.
@MainActor
final class EVStyleColorSwatchButton: NSButton {
    var swatchColor = EVStyleColorPaletteController.transparent { didSet { needsDisplay = true } }
    override var state: NSControl.StateValue { didSet { needsDisplay = true } }

    init() {
        super.init(frame: .zero)
        setButtonType(.pushOnPushOff)
        isBordered = false
        focusRingType = .exterior
        widthAnchor.constraint(equalToConstant: 32).isActive = true
        heightAnchor.constraint(equalToConstant: 32).isActive = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func draw(_ dirtyRect: NSRect) {
        let rect = bounds.insetBy(dx: 2, dy: 2)
        let outline = NSBezierPath(roundedRect: rect, xRadius: 5, yRadius: 5)
        NSGraphicsContext.saveGraphicsState()
        outline.addClip()
        NSColor.white.setFill()
        rect.fill()
        NSColor(calibratedWhite: 0.78, alpha: 1).setFill()
        for row in 0..<4 { for column in 0..<4 where (row + column) % 2 == 0 {
            NSRect(x: rect.minX + CGFloat(column) * 7, y: rect.minY + CGFloat(row) * 7, width: 7, height: 7).fill()
        } }
        swatchColor.appKitColor.setFill()
        rect.fill()
        NSGraphicsContext.restoreGraphicsState()
        NSColor.separatorColor.setStroke()
        outline.lineWidth = 1
        outline.stroke()
        if state == .on {
            let mark = NSBezierPath()
            mark.move(to: NSPoint(x: rect.minX + 6, y: rect.midY))
            mark.line(to: NSPoint(x: rect.minX + 11, y: isFlipped ? rect.maxY - 8 : rect.minY + 8))
            mark.line(to: NSPoint(x: rect.maxX - 5, y: isFlipped ? rect.minY + 7 : rect.maxY - 7))
            mark.lineCapStyle = .round
            mark.lineJoinStyle = .round
            NSColor.black.withAlphaComponent(0.65).setStroke()
            mark.lineWidth = 4
            mark.stroke()
            NSColor.white.setStroke()
            mark.lineWidth = 2
            mark.stroke()
        }
    }
}
