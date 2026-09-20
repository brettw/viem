import AppKit

/// A compact native well that opens the full Colors window with its current
/// color selected. Style and direct formatting share that window exclusively.
@MainActor
final class EVStyleColorWell: NSColorWell {
    private static weak var panelOwner: EVStyleColorWell?

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
        EVTypographyPanels.shared.stopFollowingColors()
        let panel = NSColorPanel.shared
        // Clear direct formatting's target before activation seeds the panel,
        // so merely opening it cannot change the previous owner's selection.
        panel.setTarget(nil)
        panel.setAction(nil)
        // Commit a completed color gesture once, rather than every drag sample.
        panel.isContinuous = false
        panel.showsAlpha = supportsAlpha
        // Native activation disconnects other wells and seeds the panel with
        // this well's color, including custom colors and transparency.
        activate(true)
        Self.panelOwner = self
        panel.makeKeyAndOrderFront(nil)
    }

    static func deactivatePanelOwner() {
        panelOwner?.deactivate()
        panelOwner = nil
    }

    func dismissColorControls() {
        if isActive { deactivate() }
        if Self.panelOwner === self { Self.panelOwner = nil }
    }
}
