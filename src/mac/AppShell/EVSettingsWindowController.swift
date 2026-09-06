import AppKit

@MainActor
final class EVSettingsWindowController: NSWindowController {
    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 460, height: 210),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "eVim Settings"
        window.isReleasedWhenClosed = false
        window.center()

        let title = NSTextField(labelWithString: "New Documents")
        title.font = .systemFont(ofSize: NSFont.systemFontSize, weight: .semibold)

        let fontLabel = NSTextField(labelWithString: "Default font:")
        let fontValue = NSTextField(labelWithString: "SF Pro, 14 pt")
        fontValue.font = NSFont(name: "SF Pro", size: 14) ?? .systemFont(ofSize: 14)

        let explanation = NSTextField(wrappingLabelWithString:
            "The default is recorded by the Base Document style. Per-document style editing is available from Format > Edit Styles…"
        )
        explanation.textColor = .secondaryLabelColor

        let grid = NSGridView(views: [
            [fontLabel, fontValue],
        ])
        grid.rowSpacing = 8
        grid.columnSpacing = 12
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .leading

        let stack = NSStackView(views: [title, grid, explanation])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false

        let root = NSView()
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -24),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 24),
        ])
        window.contentView = root

        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }
}
