import AppKit

@MainActor
public final class EVStatusBarView: NSView {
    public static let preferredHeight: CGFloat = 25

    private let modeLabel = NSTextField(labelWithString: "")
    private let messageLabel = NSTextField(labelWithString: "")
    private let locationLabel = NSTextField(labelWithString: "")
    private let encodingLabel = NSTextField(labelWithString: "")
    private let lineEndingLabel = NSTextField(labelWithString: "")
    private let formatLabel = NSTextField(labelWithString: "")

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

        let trailing = NSStackView(views: [locationLabel, encodingLabel, lineEndingLabel, formatLabel])
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
        encodingLabel.stringValue = state.encoding
        lineEndingLabel.stringValue = state.lineEnding
        formatLabel.stringValue = state.format

        modeLabel.setAccessibilityLabel("Mode: \(state.mode)")
        locationLabel.setAccessibilityLabel("Position: \(state.location)")
        encodingLabel.setAccessibilityLabel("Encoding: \(state.encoding)")
        lineEndingLabel.setAccessibilityLabel("Line endings: \(state.lineEnding)")
        formatLabel.setAccessibilityLabel("Format: \(state.format)")
    }
}
