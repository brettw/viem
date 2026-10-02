import AppKit

/// Only transient picker geometry lives here. Acceptance is one portable edit.
struct EVTablePickerState: Equatable {
    static let maximumColumns = 20, maximumRows = 50
    var columns = 10, rows = 10
    var selectedColumns = 0, selectedRows = 0
    var hasSelection: Bool { selectedColumns > 0 && selectedRows > 0 }

    mutating func point(column: Int, row: Int, growing: Bool) {
        guard column >= 0, row >= 0 else { selectedColumns = 0; selectedRows = 0; return }
        let c = min(Self.maximumColumns, column + 1), r = min(Self.maximumRows, row + 1)
        if growing { columns = max(columns, c); rows = max(rows, r) }
        guard c <= columns, r <= rows else { selectedColumns = 0; selectedRows = 0; return }
        selectedColumns = c; selectedRows = r
    }
    mutating func move(columns dx: Int, rows dy: Int) {
        selectedColumns = min(Self.maximumColumns, max(1, max(1, selectedColumns) + dx))
        selectedRows = min(Self.maximumRows, max(1, max(1, selectedRows) + dy))
        columns = max(columns, selectedColumns); rows = max(rows, selectedRows)
    }
}

@MainActor
final class EVInsertTableButton: NSButton {
    var openPicker: ((NSEvent?) -> Void)?
    override func mouseDown(with event: NSEvent) { if isEnabled { openPicker?(event) } }
    override func performClick(_ sender: Any?) { if isEnabled { openPicker?(nil) } }
    override func accessibilityPerformPress() -> Bool { guard isEnabled else { return false }; openPicker?(nil); return true }
}

@MainActor
private final class EVTablePickerPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

@MainActor
private final class EVTablePickerGrid: NSView {
    var state = EVTablePickerState() { didSet { needsDisplay = true } }
    static let cell: CGFloat = 21
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.controlBackgroundColor.setFill(); bounds.fill()
        for row in 0..<state.rows {
            for column in 0..<state.columns {
                let rect = NSRect(x: CGFloat(column) * Self.cell + 2, y: CGFloat(row) * Self.cell + 2,
                                  width: Self.cell - 4, height: Self.cell - 4)
                if column < state.selectedColumns && row < state.selectedRows {
                    NSColor.controlAccentColor.withAlphaComponent(0.18).setFill(); rect.fill()
                    NSColor.controlAccentColor.setStroke()
                } else { NSColor.separatorColor.setStroke() }
                let path = NSBezierPath(rect: rect); path.lineWidth = 1; path.stroke()
            }
        }
    }
}

/// A fixed-origin popup with a scrollable growing grid. One local monitor handles
/// toolbar-to-grid dragging and cell dragging without transferring mouse capture.
@MainActor
final class EVTablePickerController {
    private let panel = EVTablePickerPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    private let grid = EVTablePickerGrid()
    private let scroll = NSScrollView()
    private let title = NSTextField(labelWithString: "Insert Table")
    private let detail = NSTextField(labelWithString: "Choose columns and body rows; a header is added")
    private var eventMonitor: Any?
    private var activationObserver: NSObjectProtocol?
    private var windowObserver: NSObjectProtocol?
    private var timer: Timer?
    private var pressPoint: NSPoint?
    private var toolbarPress = false
    private var dragging = false
    private var lastPointer: NSPoint?
    private var topLeft = NSPoint.zero
    private var available = NSSize.zero
    private weak var originWindow: NSWindow?
    private weak var editor: NSView?
    private var accept: ((Int, Int) -> Void)?
    var didClose: (() -> Void)?
    private(set) var isOpen = false
    var state: EVTablePickerState { grid.state }

    init() {
        panel.isReleasedWhenClosed = false
        panel.acceptsMouseMovedEvents = true
        panel.hasShadow = true; panel.backgroundColor = .windowBackgroundColor
        panel.level = .popUpMenu
        let content = NSView(); content.wantsLayer = true; content.layer?.cornerRadius = 8; panel.contentView = content
        title.font = .systemFont(ofSize: 14, weight: .semibold)
        detail.font = .systemFont(ofSize: 10); detail.textColor = .secondaryLabelColor
        detail.lineBreakMode = .byWordWrapping; detail.maximumNumberOfLines = 2
        scroll.borderType = .noBorder; scroll.drawsBackground = false
        scroll.hasVerticalScroller = true; scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true; scroll.scrollerStyle = .overlay
        scroll.documentView = grid
        content.addSubview(title); content.addSubview(detail); content.addSubview(scroll)
        grid.setAccessibilityRole(.group)
        grid.setAccessibilityElement(true)
        grid.setAccessibilityLabel("Table dimensions")
        grid.setAccessibilityHelp("Use arrow keys to choose columns and body rows. Press Return to insert or Escape to cancel.")
    }

    func open(from button: NSView, editor: NSView, event: NSEvent?, accept: @escaping (Int, Int) -> Void) {
        if isOpen { close(); return }
        guard let window = button.window else { return }
        self.editor = editor; originWindow = window; self.accept = accept
        grid.state = EVTablePickerState()
        if event == nil { grid.state.move(columns: 0, rows: 0) }
        let buttonRect = window.convertToScreen(button.convert(button.bounds, to: nil))
        let screen = window.screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? buttonRect.insetBy(dx: -600, dy: -400)
        let initialWidth: CGFloat = 234, initialHeight: CGFloat = 278
        let x = min(max(screen.minX + 4, buttonRect.minX), screen.maxX - initialWidth - 4)
        let below = buttonRect.minY - 4
        let top = below - initialHeight >= screen.minY ? below : min(screen.maxY - 4, buttonRect.maxY + initialHeight + 4)
        topLeft = NSPoint(x: x, y: top)
        available = NSSize(width: max(100, screen.maxX - x - 4), height: max(100, top - screen.minY - 4))
        toolbarPress = event != nil; dragging = false
        pressPoint = event.map { window.convertPoint(toScreen: $0.locationInWindow) }
        isOpen = true; updateGeometry()
        window.addChildWindow(panel, ordered: .above)
        panel.makeKeyAndOrderFront(nil); panel.makeFirstResponder(grid)
        eventMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp, .leftMouseDragged, .mouseMoved, .keyDown, .scrollWheel]) { [weak self] event in
            guard let self, self.isOpen else { return event }
            return self.handle(event) ? nil : event
        }
        activationObserver = NotificationCenter.default.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
        windowObserver = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: panel, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.autoscroll() }
        }
    }

    func close(accepting: Bool = false) {
        guard isOpen else { return }
        let chosen = grid.state, completion = accept
        isOpen = false; accept = nil
        if let eventMonitor { NSEvent.removeMonitor(eventMonitor) }; eventMonitor = nil
        if let activationObserver { NotificationCenter.default.removeObserver(activationObserver) }; activationObserver = nil
        if let windowObserver { NotificationCenter.default.removeObserver(windowObserver) }; windowObserver = nil
        timer?.invalidate(); timer = nil
        originWindow?.removeChildWindow(panel); panel.orderOut(nil)
        grid.state = EVTablePickerState(); dragging = false; toolbarPress = false; lastPointer = nil
        if let editor { originWindow?.makeKey(); originWindow?.makeFirstResponder(editor) }
        didClose?()
        if accepting && chosen.hasSelection { completion?(chosen.selectedColumns, chosen.selectedRows) }
    }

    func handle(_ event: NSEvent) -> Bool {
        if event.type == .keyDown {
            switch event.keyCode {
            case 53: close()
            case 36, 76: close(accepting: true)
            case 123: grid.state.move(columns: -1, rows: 0); updateGeometry(reveal: true)
            case 124: grid.state.move(columns: 1, rows: 0); updateGeometry(reveal: true)
            case 125: grid.state.move(columns: 0, rows: 1); updateGeometry(reveal: true)
            case 126: grid.state.move(columns: 0, rows: -1); updateGeometry(reveal: true)
            default: break
            }
            return true
        }
        // AppKit may omit the window object from captured/cross-window events;
        // its originating window number still identifies the coordinate space.
        let eventWindow = event.window ?? (event.windowNumber == originWindow?.windowNumber ? originWindow : nil)
            ?? (event.windowNumber == panel.windowNumber ? panel : nil)
        let screenPoint = eventWindow?.convertPoint(toScreen: event.locationInWindow) ?? NSEvent.mouseLocation
        if event.type == .scrollWheel { return false }
        if event.type == .leftMouseDown {
            guard panel.frame.contains(screenPoint) else { close(); return false }
            let point = gridPoint(screenPoint)
            guard scrollScreenRect.contains(screenPoint), point.x >= 0, point.y >= 0 else { return true }
            toolbarPress = false; dragging = true; pressPoint = screenPoint; lastPointer = screenPoint
            pointAt(screenPoint, growing: true); return true
        }
        if event.type == .leftMouseDragged {
            if toolbarPress && !dragging, let pressPoint,
               hypot(screenPoint.x - pressPoint.x, screenPoint.y - pressPoint.y) >= 4 { dragging = true }
            if dragging { lastPointer = screenPoint; pointAt(screenPoint, growing: true) }
            return true
        }
        if event.type == .leftMouseUp {
            if dragging { pointAt(screenPoint, growing: true); close(accepting: grid.state.hasSelection) }
            else if toolbarPress { toolbarPress = false; pressPoint = nil }
            return true
        }
        if event.type == .mouseMoved {
            if scrollScreenRect.contains(screenPoint) { pointAt(screenPoint, growing: false) }
            return false
        }
        return false
    }

    var scrollScreenRect: NSRect { panel.convertToScreen(scroll.convert(scroll.bounds, to: nil)) }
    private func gridPoint(_ screen: NSPoint) -> NSPoint { grid.convert(panel.convertPoint(fromScreen: screen), from: nil) }
    private func pointAt(_ screen: NSPoint, growing: Bool) {
        let viewport = scrollScreenRect
        let point = gridPoint(screen)
        if screen.x < viewport.minX || screen.y > viewport.maxY { grid.state.point(column: -1, row: -1, growing: growing); updateGeometry(); return }
        grid.state.point(column: Int(floor(point.x / EVTablePickerGrid.cell)), row: Int(floor(point.y / EVTablePickerGrid.cell)), growing: growing)
        updateGeometry()
    }
    private func updateGeometry(reveal: Bool = false) {
        let size = NSSize(width: CGFloat(grid.state.columns) * EVTablePickerGrid.cell,
                          height: CGFloat(grid.state.rows) * EVTablePickerGrid.cell)
        let width = min(available.width, max(234, size.width + 24))
        let height = min(available.height, size.height + 68)
        panel.setFrame(NSRect(x: topLeft.x, y: topLeft.y - height, width: width, height: height), display: true)
        title.frame = NSRect(x: 12, y: height - 29, width: width - 24, height: 20)
        detail.frame = NSRect(x: 12, y: height - 54, width: width - 24, height: 24)
        scroll.frame = NSRect(x: 12, y: 12, width: width - 24, height: max(20, height - 68))
        grid.frame = NSRect(origin: .zero, size: size)
        if grid.state.hasSelection {
            let c = grid.state.selectedColumns, r = grid.state.selectedRows
            title.stringValue = "\(c) × \(r) Table"
            detail.stringValue = "\(c) \(c == 1 ? "column" : "columns"), \(r) body \(r == 1 ? "row" : "rows") + header"
            if reveal { grid.scrollToVisible(NSRect(x: CGFloat(c - 1) * EVTablePickerGrid.cell, y: CGFloat(r - 1) * EVTablePickerGrid.cell, width: EVTablePickerGrid.cell, height: EVTablePickerGrid.cell)) }
        } else { title.stringValue = "Insert Table"; detail.stringValue = "Choose columns and body rows; a header is added" }
        if grid.accessibilityValue() as? String != detail.stringValue {
            grid.setAccessibilityValue(detail.stringValue)
            NSAccessibility.post(element: grid, notification: .valueChanged)
        }
    }
    private func autoscroll() {
        guard dragging, let lastPointer else { return }
        let viewport = scrollScreenRect
        let dx: CGFloat = lastPointer.x > viewport.maxX - 12 ? 14 : lastPointer.x < viewport.minX + 12 ? -14 : 0
        let dy: CGFloat = lastPointer.y < viewport.minY + 12 ? 14 : lastPointer.y > viewport.maxY - 12 ? -14 : 0
        guard dx != 0 || dy != 0 else { return }
        let old = scroll.contentView.bounds.origin
        let next = NSPoint(x: min(max(0, old.x + dx), max(0, grid.frame.width - scroll.contentSize.width)),
                           y: min(max(0, old.y + dy), max(0, grid.frame.height - scroll.contentSize.height)))
        scroll.contentView.scroll(to: next); scroll.reflectScrolledClipView(scroll.contentView)
        pointAt(lastPointer, growing: true)
    }
}
