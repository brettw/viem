import AppKit

/// A compact native well that opens the full Colors window with its current
/// color selected.
@MainActor
final class EVStyleColorWell: NSColorWell {
    private let swatch = EVStyleColorSwatch(frame: .zero)

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
        pulldownAction = #selector(showColorPanel)
        wantsLayer = true
        layer?.masksToBounds = true
        swatch.color = color
        swatch.setAccessibilityHidden(true)
        addSubview(swatch)
    }

    override var color: NSColor {
        didSet { swatch.color = color }
    }

    override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
        swatch.frame = bounds
        // Minimal wells draw their split transparency sample in native child
        // views. Keep our display above them without intercepting native input.
        if subviews.last !== swatch { addSubview(swatch, positioned: .above, relativeTo: nil) }
    }

    override var isEnabled: Bool {
        didSet { if !isEnabled { dismissColorControls() } }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { dismissColorControls() }
    }

    @objc func showColorPanel() {
        guard isEnabled, window != nil else { return }
        let panel = NSColorPanel.shared
        // Commit a completed color gesture once, rather than every drag sample.
        panel.isContinuous = false
        panel.showsAlpha = supportsAlpha
        // Native activation disconnects other wells and seeds the panel with
        // this well's color, including custom colors and transparency.
        activate(true)
        panel.makeKeyAndOrderFront(nil)
    }

    func dismissColorControls() {
        if isActive { deactivate() }
    }
}

@MainActor
private final class EVStyleColorSwatch: NSView {
    var color: NSColor = .clear {
        didSet { needsDisplay = true }
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draw(_ dirtyRect: NSRect) {
        let insideRect = bounds
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSBezierPath(roundedRect: insideRect, xRadius: insideRect.height / 2,
                     yRadius: insideRect.height / 2).addClip()
        NSColor.white.setFill()
        insideRect.fill(using: .sourceOver)
        NSColor(srgbRed: 0.7, green: 0.7, blue: 0.7, alpha: 1).setFill()
        let squareSize: CGFloat = 5
        for row in 0..<Int(ceil(insideRect.height / squareSize)) {
            for column in 0..<Int(ceil(insideRect.width / squareSize)) where (row + column) % 2 == 1 {
                NSRect(x: insideRect.minX + CGFloat(column) * squareSize,
                       y: insideRect.minY + CGFloat(row) * squareSize,
                       width: squareSize, height: squareSize).fill(using: .sourceOver)
            }
        }
        color.setFill()
        insideRect.fill(using: .sourceOver)
    }
}
