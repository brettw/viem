import AppKit

/// Popup-local draft state. Only dimensions are authored; the lock and the
/// last edited axis are reset whenever an image editor is opened.
@MainActor
final class EVImageSizeControls: NSStackView, NSTextFieldDelegate {
    let disclosure = NSButton(title: "Specify size", target: nil, action: nil)
    let widthField = NSTextField()
    let heightField = NSTextField()
    let lockButton = NSButton()
    let useImageSizeButton = NSButton(title: "Use image size", target: nil, action: nil)
    private let details = NSStackView()
    private var natural: NSSize?
    private var originalWidth: UInt32?
    private var originalHeight: UInt32?
    private var changed = false
    private var fieldsEdited = false
    private var inferLock = true
    private var lastEditedHeight = false
    private var expandedOnce = false
    private(set) var expanded = false
    var didChange: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        orientation = .vertical; alignment = .leading; spacing = 8
        detachesHiddenViews = true
        widthAnchor.constraint(equalToConstant: 360).isActive = true
        disclosure.isBordered = false; disclosure.alignment = .left
        disclosure.imagePosition = .imageLeading
        disclosure.target = self; disclosure.action = #selector(toggleExpanded)
        disclosure.setAccessibilityLabel("Specify image size")
        addArrangedSubview(disclosure)
        for (field, label) in [(widthField, "Image width in pixels"), (heightField, "Image height in pixels")] {
            field.font = .systemFont(ofSize: NSFont.systemFontSize)
            field.widthAnchor.constraint(equalToConstant: 70).isActive = true
            field.delegate = self; field.setAccessibilityLabel(label)
        }
        lockButton.setButtonType(.toggle); lockButton.bezelStyle = .accessoryBarAction
        lockButton.imagePosition = .imageOnly
        lockButton.target = self; lockButton.action = #selector(toggleLock)
        lockButton.widthAnchor.constraint(equalToConstant: 26).isActive = true
        let row = NSStackView(views: [NSTextField(labelWithString: "Width"), widthField,
            NSTextField(labelWithString: "× Height"), heightField, lockButton])
        row.orientation = .horizontal; row.alignment = .centerY; row.spacing = 6
        details.orientation = .vertical; details.alignment = .leading; details.spacing = 8
        details.addArrangedSubview(row)
        useImageSizeButton.bezelStyle = .rounded
        useImageSizeButton.target = self; useImageSizeButton.action = #selector(useImageSize)
        details.addArrangedSubview(useImageSizeButton)
        addArrangedSubview(details)
        reset(width: nil, height: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func reset(width: UInt32?, height: UInt32?) {
        originalWidth = width; originalHeight = height; natural = nil
        changed = false; fieldsEdited = false; inferLock = true; lastEditedHeight = false
        expanded = width != nil || height != nil; expandedOnce = expanded
        widthField.stringValue = width.map(String.init) ?? ""
        heightField.stringValue = height.map(String.init) ?? ""
        lockButton.state = .on
        refresh()
    }

    func setNaturalSize(_ size: NSSize?) {
        natural = size
        if let size {
            if inferLock {
                if let width = originalWidth, let height = originalHeight {
                    let matches = abs(Double(height) - Double(width) * size.height / size.width) <= 1
                        || abs(Double(width) - Double(height) * size.width / size.height) <= 1
                    lockButton.state = matches ? .on : .off
                } else { lockButton.state = .on }
                inferLock = false
            }
            if expandedOnce {
                if !fieldsEdited, widthField.stringValue.isEmpty, heightField.stringValue.isEmpty { fillNatural() }
                else if !fieldsEdited, widthField.stringValue.isEmpty, let height = Self.value(heightField) {
                    widthField.stringValue = scaled(Double(height) * size.width / size.height)
                } else if !fieldsEdited, heightField.stringValue.isEmpty, let width = Self.value(widthField) {
                    heightField.stringValue = scaled(Double(width) * size.height / size.width)
                } else if changed, lockButton.state == .on { matchAspect() }
            }
        }
        refresh(); didChange?()
    }

    var dimensions: (width: UInt32?, height: UInt32?) {
        guard expandedOnce else { return (nil, nil) }
        // Merely displaying a derived second dimension preserves one-axis HTML.
        if !changed { return (originalWidth, originalHeight) }
        return (Self.value(widthField), Self.value(heightField))
    }

    var isValid: Bool {
        guard expandedOnce, changed else { return true }
        let fields = [widthField, heightField]
        return fields.allSatisfy { $0.stringValue.isEmpty || Self.value($0) != nil }
    }

    private static func value(_ field: NSTextField) -> UInt32? {
        let text = field.stringValue
        guard !text.isEmpty, text.utf8.allSatisfy({ (48...57).contains($0) }),
              let value = UInt32(text), value > 0 else { return nil }
        return value
    }

    private func scaled(_ value: Double) -> String {
        // Keep overflow visible and invalid instead of silently changing the
        // aspect ratio. Natural sizes and uint32 input keep this below UInt64.
        String(UInt64(max(1, value.rounded())))
    }

    private func fillNatural() {
        guard let natural else { return }
        widthField.stringValue = scaled(natural.width)
        heightField.stringValue = scaled(natural.height)
    }

    private func matchAspect() {
        guard let natural else { return }
        if lastEditedHeight {
            if let height = Self.value(heightField) {
                widthField.stringValue = scaled(Double(height) * natural.width / natural.height)
            }
        } else if let width = Self.value(widthField) {
            heightField.stringValue = scaled(Double(width) * natural.height / natural.width)
        }
    }

    @objc private func toggleExpanded() {
        expanded.toggle()
        if expanded, !expandedOnce {
            expandedOnce = true; changed = true; lockButton.state = .on; inferLock = false
            fillNatural()
        }
        refresh(); didChange?()
    }

    @objc private func toggleLock() {
        inferLock = false
        if lockButton.state == .on { changed = true; matchAspect() }
        refresh(); didChange?()
    }

    @objc private func useImageSize() {
        guard natural != nil else { return }
        changed = true; fillNatural(); didChange?()
    }

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField,
              field === widthField || field === heightField else { return }
        changed = true; fieldsEdited = true; inferLock = false; lastEditedHeight = field === heightField
        if lockButton.state == .on { matchAspect() }
        didChange?()
    }

    private func refresh() {
        details.isHidden = !expanded
        disclosure.image = NSImage(systemSymbolName: expanded ? "chevron.down" : "chevron.right", accessibilityDescription: nil)
        disclosure.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        lockButton.image = NSImage(systemSymbolName: lockButton.state == .on ? "lock.fill" : "lock.open", accessibilityDescription: nil)
        let label = lockButton.state == .on ? "Unlock image aspect ratio" : "Lock image aspect ratio"
        lockButton.toolTip = label; lockButton.setAccessibilityLabel(label)
        lockButton.isEnabled = natural != nil
        useImageSizeButton.isEnabled = natural != nil
    }
}
