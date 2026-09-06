import AppKit
import EvimAppShell

/// Native text selection and Copy remain available without mutating command output.
@MainActor
final class EVCommandOutputBar: NSView {
    private let scroll = NSScrollView()
    let textView = EVCommandOutputTextView()
    private let closeButton = NSButton()
    var close: (() -> Void)?
    var beginCommand: (() -> Void)? {
        didSet { textView.beginCommand = beginCommand }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        textView.textContainerInset = NSSize(width: 10, height: 7)
        textView.autoresizingMask = [.width]
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.textContainer?.widthTracksTextView = true
        scroll.documentView = textView
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        addSubview(scroll)
        closeButton.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close command output")
        closeButton.title = ""
        closeButton.imagePosition = .imageOnly
        closeButton.bezelStyle = .inline
        closeButton.isBordered = false
        closeButton.target = self
        closeButton.action = #selector(dismiss(_:))
        closeButton.setAccessibilityLabel("Close command output")
        addSubview(closeButton)
    }
    required init?(coder: NSCoder) { nil }
    override var isFlipped: Bool { true }
    override func layout() {
        super.layout()
        scroll.frame = NSRect(x: 0, y: 1, width: max(0, bounds.width - 32), height: max(0, bounds.height - 1))
        textView.frame.size.width = scroll.contentSize.width
        closeButton.frame = NSRect(x: max(0, bounds.width - 29), y: 5, width: 24, height: 24)
    }
    func show(_ value: String) {
        if textView.string != value { textView.string = value; textView.scrollRangeToVisible(NSRange(location: 0, length: 0)) }
        let theme = EVThemeStore.shared.theme
        closeButton.contentTintColor = theme.foreground.color
        closeButton.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close command output")?.withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 12, weight: .medium))
        needsLayout = true
        layoutSubtreeIfNeeded()
        textView.textColor = theme.foreground.color
        textView.backgroundColor = theme.background.color
        layer?.backgroundColor = theme.background.color.cgColor
        layer?.borderColor = NSColor.separatorColor.cgColor
        layer?.borderWidth = 1
    }
    @objc private func dismiss(_ sender: Any?) { close?() }
}

@MainActor
final class EVCommandOutputTextView: NSTextView {
    var beginCommand: (() -> Void)?
    override func keyDown(with event: NSEvent) {
        if event.characters == ":", event.modifierFlags.intersection([.command, .control, .option]).isEmpty {
            beginCommand?()
        } else { super.keyDown(with: event) }
    }
}
