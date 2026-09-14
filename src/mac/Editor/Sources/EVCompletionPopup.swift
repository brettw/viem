import AppKit

/// Native presentation only. Candidate order and selection belong to the core;
/// this window neither becomes a key window nor interprets completion keys.
@MainActor
final class EVCompletionPopup: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    private let panel: EVCompletionPanel
    private let table = EVCompletionTableView()
    private let scroll = EVCompletionScrollView()
    private let status = NSTextField(labelWithString: "")
    private let measurementCell = EVCompletionWordCell()
    private var rightToLeft = false
    private(set) var items: [String] = []
    private(set) var selectedIndex: Int?
    private weak var owner: NSWindow?
    private let rowHeight: CGFloat = 24

    var isVisible: Bool { panel.isVisible }
    var nativeSelectedRow: Int { table.selectedRow }
    var window: NSWindow { panel }

    override init() {
        panel = EVCompletionPanel(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        super.init()
        panel.isReleasedWhenClosed = false
        panel.hasShadow = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hidesOnDeactivate = true
        panel.isFloatingPanel = true
        // Pointer input continues to the editor. A native row must never
        // introduce a selection that was not published by the core.
        panel.ignoresMouseEvents = true
        panel.setAccessibilityLabel("Word completions")

        let backdrop = NSVisualEffectView()
        backdrop.material = .popover
        backdrop.blendingMode = .behindWindow
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = 7
        backdrop.layer?.masksToBounds = true
        panel.contentView = backdrop

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("word"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = rowHeight
        table.intercellSpacing = .zero
        table.backgroundColor = .clear
        table.style = .plain
        table.selectionHighlightStyle = .regular
        table.allowsEmptySelection = true
        table.allowsMultipleSelection = false
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = self
        table.delegate = self
        table.setAccessibilityLabel("Word completions")
        table.setAccessibilityHelp("Control-N selects the next completion. Control-P selects the previous completion.")

        scroll.documentView = table
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        backdrop.addSubview(scroll)
        status.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        status.textColor = .secondaryLabelColor
        backdrop.addSubview(status)
    }

    func show(items: [String], selectedIndex: Int?, searching: Bool, truncated: Bool,
              at anchor: NSRect, in owner: NSWindow, rightToLeft: Bool) {
        guard !items.isEmpty || searching else { hide(); return }
        let changedItems = self.items != items
        let changedSelection = self.selectedIndex != selectedIndex
        let changedDirection = self.rightToLeft != rightToLeft
        self.items = items
        self.selectedIndex = selectedIndex
        self.rightToLeft = rightToLeft
        let direction: NSUserInterfaceLayoutDirection = rightToLeft ? .rightToLeft : .leftToRight
        panel.contentView?.userInterfaceLayoutDirection = direction
        scroll.userInterfaceLayoutDirection = direction
        scroll.contentView.userInterfaceLayoutDirection = direction
        table.userInterfaceLayoutDirection = direction
        status.userInterfaceLayoutDirection = direction
        status.alignment = rightToLeft ? .right : .left
        status.baseWritingDirection = rightToLeft ? .rightToLeft : .leftToRight
        if changedItems || changedDirection { table.reloadData() }
        if let selectedIndex, items.indices.contains(selectedIndex) {
            table.selectRowIndexes(IndexSet(integer: selectedIndex), byExtendingSelection: false)
        } else {
            table.deselectAll(nil)
        }

        status.stringValue = searching ? "Searching…" : truncated ? "Results limited" : ""
        status.isHidden = status.stringValue.isEmpty
        let footer: CGFloat = status.isHidden ? 0 : 22
        // NSTableView materializes visible rows only. Keep width independent
        // of the number of results so incremental arrivals do not synchronously
        // measure the entire candidate batch on the UI thread.
        let width: CGFloat = 320
        let height = CGFloat(min(8, items.count)) * rowHeight + footer + 8
        let visibleFrame = owner.screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? owner.frame
        let size = NSSize(width: min(width, visibleFrame.width), height: min(height, visibleFrame.height))
        panel.appearance = owner.effectiveAppearance
        panel.setContentSize(size)
        scroll.frame = NSRect(x: 4, y: footer + 4, width: size.width - 8, height: max(0, size.height - footer - 8))
        scroll.tile()
        table.frame.size.width = scroll.contentSize.width
        table.sizeLastColumnToFit()
        if let selectedIndex, changedItems || changedSelection { table.scrollRowToVisible(selectedIndex) }
        status.frame = NSRect(x: 10, y: 5, width: size.width - 20, height: 17)
        panel.contentView?.layoutSubtreeIfNeeded()
        let frame = Self.frame(for: size, maximumHeight: 8 * rowHeight + 22 + 8, anchor: anchor, textInset: textInset(),
                               rightToLeft: rightToLeft, visibleFrame: visibleFrame)
        if self.owner !== owner {
            self.owner?.removeChildWindow(panel)
            self.owner = owner
            owner.addChildWindow(panel, ordered: .above)
        }
        panel.orderFront(nil)
        // Initial ordering can adjust a native panel frame; place it afterward
        // so the first result and later selections use the same text anchor.
        panel.setFrame(frame, display: false)
        if changedSelection {
            NSAccessibility.post(element: table, notification: .selectedRowsChanged)
        }
    }

    func hide() {
        panel.orderOut(nil)
        owner?.removeChildWindow(panel)
        owner = nil
    }

    /// Measure through the native scroll/column/cell geometry, including the
    /// label cell's own drawing inset. The prototype has the same constraints
    /// as visible rows, so placement is also stable before any result arrives.
    private func textInset() -> CGFloat {
        let column = table.rect(ofColumn: 0)
        measurementCell.frame = NSRect(x: 0, y: 0, width: column.width, height: rowHeight)
        measurementCell.configure(text: "", rightToLeft: rightToLeft)
        measurementCell.layoutSubtreeIfNeeded()
        let text = measurementCell.textField!
        let drawing = text.cell!.drawingRect(forBounds: text.bounds)
        let local = text.convert(drawing, to: measurementCell).offsetBy(dx: column.minX, dy: 0)
        let content = table.convert(local, to: panel.contentView)
        return rightToLeft ? panel.contentView!.bounds.maxX - content.maxX : content.minX
    }

    static func frame(for size: NSSize, maximumHeight: CGFloat, anchor: NSRect, textInset: CGFloat,
                      rightToLeft: Bool, visibleFrame: NSRect) -> NSRect {
        let width = min(size.width, visibleFrame.width)
        let height = min(size.height, visibleFrame.height)
        let below = anchor.minY - height - 3
        // Results may arrive incrementally and the status footer may disappear.
        // Decide the side using the largest popup, so those updates cannot move
        // it from one side of an unchanged word anchor to the other.
        let fitsBelow = anchor.minY - min(maximumHeight, visibleFrame.height) - 3 >= visibleFrame.minY
        let preferredY = fitsBelow ? below : anchor.maxY + 3
        let preferredX = rightToLeft ? anchor.maxX - width + textInset : anchor.minX - textInset
        return NSRect(
            x: min(max(preferredX, visibleFrame.minX), visibleFrame.maxX - width),
            y: min(max(preferredY, visibleFrame.minY), visibleFrame.maxY - height),
            width: width,
            height: height
        )
    }

    func numberOfRows(in tableView: NSTableView) -> Int { items.count }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        EVCompletionRowView()
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard items.indices.contains(row) else { return nil }
        let cell = tableView.makeView(withIdentifier: EVCompletionWordCell.reuseIdentifier, owner: self) as? EVCompletionWordCell
            ?? EVCompletionWordCell()
        cell.configure(text: items[row], rightToLeft: rightToLeft)
        return cell
    }
}

private final class EVCompletionWordCell: NSTableCellView {
    static let reuseIdentifier = NSUserInterfaceItemIdentifier("completionWord")

    init() {
        super.init(frame: .zero)
        identifier = Self.reuseIdentifier
        let text = NSTextField(labelWithString: "")
        text.font = .systemFont(ofSize: 13)
        text.lineBreakMode = .byTruncatingTail
        text.translatesAutoresizingMaskIntoConstraints = false
        addSubview(text)
        textField = text
        NSLayoutConstraint.activate([
            text.leftAnchor.constraint(equalTo: leftAnchor, constant: 6),
            text.rightAnchor.constraint(equalTo: rightAnchor, constant: -6),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(text: String, rightToLeft: Bool) {
        userInterfaceLayoutDirection = rightToLeft ? .rightToLeft : .leftToRight
        textField?.userInterfaceLayoutDirection = userInterfaceLayoutDirection
        textField?.baseWritingDirection = rightToLeft ? .rightToLeft : .leftToRight
        textField?.alignment = rightToLeft ? .right : .left
        textField?.stringValue = text
    }
}

private final class EVCompletionPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

private final class EVCompletionScrollView: NSScrollView {
    override func tile() {
        super.tile()
        // AppKit's scroll view keeps the vertical scroller on the system's
        // side even when this popup alone changes direction. Mirror its native
        // tiled frames; retain AppKit's widths and overlay/legacy behavior.
        guard let scroller = verticalScroller else { return }
        let wantsScrollerOnLeft = userInterfaceLayoutDirection == .rightToLeft
        let scrollerIsOnLeft = scroller.frame.midX < bounds.midX
        guard wantsScrollerOnLeft != scrollerIsOnLeft else { return }
        scroller.frame.origin.x = bounds.minX + bounds.maxX - scroller.frame.maxX
        contentView.frame.origin.x = bounds.minX + bounds.maxX - contentView.frame.maxX
    }
}

private final class EVCompletionTableView: NSTableView {
    override var acceptsFirstResponder: Bool { false }

    // Accessibility reports native rows and the selected item, but edits must
    // still pass through portable completion commands.
    override func setAccessibilitySelectedRows(_ selectedRows: [any NSAccessibilityRow]) {}
    override func setAccessibilitySelectedChildren(_ selectedChildren: [Any]?) {}

    override func isAccessibilitySelectorAllowed(_ selector: Selector) -> Bool {
        if selector == #selector(setAccessibilitySelectedRows(_:))
            || selector == #selector(setAccessibilitySelectedChildren(_:)) { return false }
        return super.isAccessibilitySelectorAllowed(selector)
    }
}

private final class EVCompletionRowView: NSTableRowView {
    override func setAccessibilitySelected(_ selected: Bool) {}

    override func isAccessibilitySelectorAllowed(_ selector: Selector) -> Bool {
        if selector == #selector(setAccessibilitySelected(_:)) { return false }
        return super.isAccessibilitySelectorAllowed(selector)
    }
}
