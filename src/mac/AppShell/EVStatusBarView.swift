import AppKit

@MainActor
public final class EVStatusBarView: NSView {
    public static let preferredHeight: CGFloat = 25

    private let modeLabel = NSTextField(labelWithString: "")
    private let messageLabel = NSTextField(labelWithString: "")
    private let locationLabel = NSTextField(labelWithString: "")
    public var optionDidChange: ((EVStatusBarOption) -> Void)?
    private let encodingSelect = EVStatusSelect()
    private let lineEndingSelect = EVStatusSelect()
    private let formatSelect = EVStatusSelect()

    public override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityRole(.group)
        setAccessibilityLabel("Editor status")

        let separator = NSBox()
        separator.boxType = .separator
        separator.translatesAutoresizingMaskIntoConstraints = false

        modeLabel.font = .systemFont(ofSize: 11, weight: .semibold)
        modeLabel.setContentHuggingPriority(.required, for: .horizontal)
        messageLabel.textColor = .secondaryLabelColor
        messageLabel.lineBreakMode = .byTruncatingTail
        messageLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        encodingSelect.configure(label: "Encoding", options: [
            ("UTF-8", .encoding(1)), ("Latin-1", .encoding(2)),
            ("UTF-16 LE", .encoding(3)), ("UTF-16 BE", .encoding(4)),
        ])
        lineEndingSelect.configure(label: "Line endings", options: [
            ("LF", .lineEnding(1)), ("CRLF", .lineEnding(2)), ("CR", .lineEnding(3)),
        ])
        formatSelect.configure(label: "Format", options:
            [EVSourceFormat.plainText, .markdownSource, .markdown, .html, .rtf].map {
                ($0.displayName, .format($0))
            }
        )
        for select in [encodingSelect, lineEndingSelect, formatSelect] {
            select.didChoose = { [weak self] option in self?.optionDidChange?(option) }
        }
        let trailing = NSStackView(views: [locationLabel, encodingSelect, lineEndingSelect, formatSelect])
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
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.preferredHeight),
            separator.leadingAnchor.constraint(equalTo: leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: trailingAnchor),
            separator.topAnchor.constraint(equalTo: topAnchor),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            row.topAnchor.constraint(equalTo: separator.bottomAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        apply(EVStatusBarState())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    public func apply(_ state: EVStatusBarState) {
        modeLabel.stringValue = state.mode
        messageLabel.stringValue = state.message
        locationLabel.stringValue = state.location
        encodingSelect.selectItem(withTitle: state.encoding)
        lineEndingSelect.selectItem(withTitle: state.lineEnding)
        formatSelect.selectItem(withTitle: state.format)
        for select in [encodingSelect, lineEndingSelect, formatSelect] {
            select.invalidateIntrinsicContentSize()
        }
        lineEndingSelect.isEnabled = state.format != EVSourceFormat.rtf.displayName
        encodingSelect.isEnabled = state.format != EVSourceFormat.rtf.displayName

        modeLabel.setAccessibilityLabel("Mode: \(state.mode)")
        locationLabel.setAccessibilityLabel("Position: \(state.location)")
        encodingSelect.setAccessibilityLabel("Encoding: \(state.encoding)")
        lineEndingSelect.setAccessibilityLabel("Line endings: \(state.lineEnding)")
        formatSelect.setAccessibilityLabel("Format: \(state.format)")
    }
}

/// Native select tracking and keyboard accessibility with unobtrusive status chrome.
@MainActor
private final class EVStatusSelect: NSPopUpButton {
    var didChoose: ((EVStatusBarOption) -> Void)?
    private var options: [EVStatusBarOption] = []
    private var hoverTracking: NSTrackingArea?

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

    override var intrinsicContentSize: NSSize {
        let textWidth = (title as NSString).size(withAttributes: [.font: font ?? NSFont.systemFont(ofSize: 11)]).width
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
        if isEnabled { layer?.backgroundColor = NSColor.labelColor.withAlphaComponent(0.08).cgColor }
    }

    override func mouseExited(with event: NSEvent) { layer?.backgroundColor = nil }

    @objc private func choose(_ sender: Any?) {
        guard options.indices.contains(indexOfSelectedItem) else { return }
        didChoose?(options[indexOfSelectedItem])
    }
}

@MainActor
private final class EVStatusSelectCell: NSPopUpButtonCell {
    override var cellSize: NSSize {
        var size = super.cellSize
        size.width += 10
        return size
    }

    override func draw(withFrame cellFrame: NSRect, in controlView: NSView) {
        var titleFrame = cellFrame
        titleFrame.size.width -= 10
        super.draw(withFrame: titleFrame, in: controlView)
        let x = cellFrame.maxX - 6
        let y = cellFrame.midY
        let triangle = NSBezierPath()
        triangle.move(to: NSPoint(x: x - 2.5, y: y - 1.5))
        triangle.line(to: NSPoint(x: x + 2.5, y: y - 1.5))
        triangle.line(to: NSPoint(x: x, y: y + 1.5))
        triangle.close()
        (isEnabled ? NSColor.secondaryLabelColor : .disabledControlTextColor).setFill()
        triangle.fill()
    }
}
