import AppKit
import CViemCore

extension ViemLogicalSelectionIdentityV1 {
    func isSameSelection(as other: Self) -> Bool {
        document_id == other.document_id && document_revision == other.document_revision
            && view_id == other.view_id && text_start == other.text_start && text_end == other.text_end
            && kind == other.kind && withUnsafeBytes(of: state_identity) { left in
                withUnsafeBytes(of: other.state_identity) { left.elementsEqual($0) }
            }
    }
}

private final class EVTableMenuAction: NSObject {
    let context: ViemTableContextV1
    let action: UInt32, alignment: UInt32
    init(_ context: ViemTableContextV1, _ action: UInt32, _ alignment: UInt32 = 0) {
        self.context = context; self.action = action; self.alignment = alignment
    }
}

@MainActor
private final class EVAccessibleTableCell: NSAccessibilityElement {
    var readValue: (() -> String?)?
    var focusCell: (() -> Bool)?
    override func accessibilityPerformPress() -> Bool { focusCell?() ?? false }
    override func accessibilityValue() -> Any? { readValue?() ?? "" }
}

@MainActor
final class EVTableInteraction: NSObject, NSMenuDelegate {
    private enum WidgetKind {
        case column, rowLeft, rowRight, rowSelection, columnSelection
        var isSelection: Bool { self == .rowSelection || self == .columnSelection }
    }
    private struct ColumnGesture { let table: UInt64, anchor: UInt64, rows: UInt64 }
    private weak var editor: EVEditorView?
    private let widget = NSStackView()
    private var target: ViemTableContextV1?
    private var targetViewport = NSPoint.zero
    private var targetIdentity: ViemLayoutSnapshotIdentityV1?
    private var activation = NSRect.zero
    private var targetKind: WidgetKind?
    private var popupTracking = false
    private var dragAnchor: ViemTableCellV1?
    private var dragIdentity: ViemLayoutSnapshotIdentityV1?
    private var dragOrigin = NSPoint.zero
    private var cellDrag = false
    private var columnGesture: ColumnGesture?
    /// Whether a selection of every cell shows column rather than row controls.
    private var prefersColumnControls = false
    private var surface: EVEditorSurfaceController? { editor?.surface }
    var isSelectingColumns: Bool { columnGesture != nil }

    /// A centered down arrow in the style of the system arrow: black with a
    /// white outline. Its tip is the hot spot.
    static let columnSelectionCursor: NSCursor = {
        let image = NSImage(size: NSSize(width: 15, height: 22), flipped: true) { _ in
            let path = NSBezierPath()
            for (index, point) in [(5.0, 1.5), (10, 1.5), (10, 11), (13.5, 11), (7.5, 19), (1.5, 11), (5, 11)].enumerated() {
                let point = NSPoint(x: point.0, y: point.1)
                if index == 0 { path.move(to: point) } else { path.line(to: point) }
            }
            path.close()
            path.lineWidth = 2
            NSColor.white.setStroke(); path.stroke()
            NSColor.black.setFill(); path.fill()
            return true
        }
        return NSCursor(image: image, hotSpot: NSPoint(x: 7.5, y: 19))
    }()

    init(editor: EVEditorView) {
        self.editor = editor
        super.init()
        widget.orientation = .horizontal; widget.spacing = 1
        widget.wantsLayer = true; widget.layer?.cornerRadius = 5
        widget.edgeInsets = NSEdgeInsets(top: 3, left: 3, bottom: 3, right: 3)
        widget.isHidden = true; editor.addSubview(widget)
    }

    func invalidate() {
        if targetKind?.isSelection != true {
            if let identity = targetIdentity, surface?.layoutSnapshot?.info.identity.isSameLayout(as: identity) != true || editor?.viewportOrigin != targetViewport { hide() }
            if let expected = target?.selection,
               let current = try? surface?.session?.listSelection(), !expected.isSameSelection(as: current) { hide() }
        }
        refreshSelectionWidget()
    }
    /// Hides hover controls. Controls for selected rows or columns follow the
    /// selection instead.
    func hide() { if targetKind?.isSelection != true { dismiss() } }
    private func dismiss() { if !popupTracking { widget.isHidden = true; target = nil; targetIdentity = nil; targetKind = nil } }

    func ownsPointerInteraction(at point: NSPoint) -> Bool {
        guard !widget.isHidden, let identity = targetIdentity,
              surface?.layoutSnapshot?.info.identity.isSameLayout(as: identity) == true,
              editor?.viewportOrigin == targetViewport else { return false }
        return widget.frame.contains(point)
    }

    private static func ownsColumn(_ point: NSPoint, cell: NSRect, table: NSRect) -> Bool {
        point.y >= table.minY - 3 && point.y <= table.minY + 1 && point.x >= table.minX && point.x <= table.maxX
            && point.x >= cell.minX && (point.x < cell.maxX || (point.x == cell.maxX && cell.maxX == table.maxX))
    }

    func moved(to point: NSPoint) {
        guard let editor, let surface, let snapshot = surface.layoutSnapshot, surface.backend.sourceFormat == .markdown,
              editor.bounds.contains(point) else { hide(); return }
        if !widget.isHidden && (widget.frame.contains(point) || popupTracking || targetKind?.isSelection == true) { return }
        for cell in snapshot.tableCells {
            let table = editor.viewRect(cell.table_rect), rect = editor.viewRect(cell.rect)
            let topStrip = point.y >= table.minY - 3 && point.y <= table.minY + 1
                && point.x >= table.minX && point.x <= table.maxX
            let column = Self.ownsColumn(point, cell: rect, table: table)
            let inRow = !topStrip && point.y >= rect.minY
                && (point.y < rect.maxY || (point.y == rect.maxY && rect.maxY == table.maxY))
            // The left zone is the row's selection margin; the right zone
            // reaches the right edge of the editor.
            let left = inRow && point.x < table.minX + 1
            let right = inRow && point.x >= table.maxX - 1
            guard column || left || right else { continue }
            let kind: WidgetKind = column ? .column : left ? .rowLeft : .rowRight
            if let target, target.table_id == cell.table_id,
               column ? target.column == cell.column : target.row == cell.row {
                if targetKind == kind { return }
                // A flipped row widget's bridge crosses the opposite gutter.
                // Preserve its controls while entering from either side.
                if !column && targetKind != .column,
                   widget.frame.union(activation).insetBy(dx: -3, dy: -3).contains(point) { return }
            }
            guard let context = try? surface.session?.tableContext(at: cell.text_start, documentID: snapshot.info.identity.document_id, revision: snapshot.info.identity.document_revision), context.flags & 2 != 0 else { break }
            show(context, kind: kind, cell: rect, table: table, identity: snapshot.info.identity)
            return
        }
        if !widget.isHidden && widget.frame.union(activation).insetBy(dx: -3, dy: -3).contains(point) { return }
        hide()
    }

    /// Selected complete rows or columns show their controls at the selection's
    /// active row or column, independent of hovering.
    private func refreshSelectionWidget() {
        guard !popupTracking else { return }
        guard let editor, let surface, let session = surface.session, surface.backend.sourceFormat == .markdown,
              !editor.gutterSelectionDragging, columnGesture == nil, let snapshot = surface.layoutSnapshot,
              let selection = try? session.tableSelection(), selection.active != 0,
              selection.document_revision == snapshot.info.identity.document_revision,
              let context = try? session.tableContext(), context.table_id == selection.table_id else {
            if targetKind?.isSelection == true { dismiss() }
            return
        }
        let rows = context.flags & UInt32(VIEM_TABLE_ROW_SELECTION) != 0
        let columns = context.flags & UInt32(VIEM_TABLE_COLUMN_SELECTION) != 0
        let kind: WidgetKind = columns && (!rows || prefersColumnControls) ? .columnSelection : .rowSelection
        let cells = snapshot.tableCells.filter { $0.table_id == selection.table_id
            && (kind == .rowSelection ? $0.row == selection.active_row : $0.column == selection.active_column) }
        guard rows || columns, let cell = cells.min(by: { $0.row < $1.row }) else {
            if targetKind?.isSelection == true { dismiss() }
            return
        }
        if targetKind == kind, !widget.isHidden, let target, target.selection.isSameSelection(as: context.selection),
           target.row == context.row, target.column == context.column,
           targetIdentity?.isSameLayout(as: snapshot.info.identity) == true, editor.viewportOrigin == targetViewport { return }
        show(context, kind: kind, cell: editor.viewRect(cell.rect), table: editor.viewRect(cell.table_rect), identity: snapshot.info.identity)
    }

    private static func rowsName(_ context: ViemTableContextV1) -> String {
        context.row_end - context.row > 1 ? "rows \(context.row + 1)–\(context.row_end)"
            : context.row == 0 ? "header row" : "row \(context.row + 1)"
    }
    private static func columnsName(_ context: ViemTableContextV1) -> String {
        context.column_end - context.column > 1 ? "columns \(context.column + 1)–\(context.column_end)" : "column \(context.column + 1)"
    }
    private static func alignmentName(_ context: ViemTableContextV1) -> String {
        let columns = columnsName(context)
        return columns.prefix(1).uppercased() + columns.dropFirst() + " alignment"
    }

    private func show(_ context: ViemTableContextV1, kind: WidgetKind, cell: NSRect, table: NSRect, identity: ViemLayoutSnapshotIdentityV1) {
        guard let editor else { return }
        let column = kind == .column || kind == .columnSelection
        target = context; targetKind = kind; targetIdentity = identity; targetViewport = editor.viewportOrigin
        for view in widget.arrangedSubviews { widget.removeArrangedSubview(view); view.removeFromSuperview() }
        let header = context.flags & UInt32(VIEM_TABLE_HEADER) != 0
        let rows = Self.rowsName(context), columns = Self.columnsName(context)
        var actions: [(String, String, UInt32)] = []
        switch kind {
        case .column, .columnSelection:
            actions.append(("Insert column left of \(columns)", "+←", 4))
            actions.append((Self.alignmentName(context), "", 7))
            actions += [("Delete \(columns)", "−", 6), ("Insert column right of \(columns)", "+→", 5)]
        case .rowSelection:
            if !header { actions.append(("Insert row above \(rows)", "+↑", 1)) }
            actions += [("Delete \(rows)", "−", 3), ("Insert row below \(rows)", "+↓", 2)]
        case .rowLeft, .rowRight:
            // The header row can only gain a body row below it.
            actions = header ? [("Insert row below row 1", "+↓", 2)]
                : [("Insert row above \(rows)", "+↑", 1), ("Delete \(rows)", "−", 3), ("Insert row below \(rows)", "+↓", 2)]
        }
        for (label, glyph, action) in actions {
            let button = NSButton(title: glyph, target: self, action: action == 7 ? #selector(alignmentPopup(_:)) : #selector(widgetAction(_:)))
            button.tag = Int(action); button.bezelStyle = .accessoryBarAction; button.controlSize = .small
            button.refusesFirstResponder = true; button.toolTip = label; button.setAccessibilityLabel(label)
            button.widthAnchor.constraint(equalToConstant: 27).isActive = true; button.heightAnchor.constraint(equalToConstant: 24).isActive = true
            if action == 7 { button.image = NSImage(systemSymbolName: context.alignment == 2 ? "text.aligncenter" : context.alignment == 3 ? "text.alignright" : "text.alignleft", accessibilityDescription: label) }
            widget.addArrangedSubview(button)
        }
        let width = CGFloat(actions.count * 28 + 6), height: CGFloat = 30
        let leftX = table.minX - width - 4, rightX = table.maxX + 4
        var x = column ? cell.midX - width / 2 : kind == .rowRight ? rightX : leftX
        var y = column ? table.minY - height - 4 : cell.midY - height / 2
        if kind != .rowRight && !column && x < editor.bounds.minX && rightX + width <= editor.bounds.maxX { x = rightX }
        if kind == .rowRight && x + width > editor.bounds.maxX && leftX >= editor.bounds.minX { x = leftX }
        if y < editor.bounds.minY { y = cell.maxY + 4 }
        x = max(editor.bounds.minX, min(x, editor.bounds.maxX - width))
        y = max(editor.bounds.minY, min(y, editor.bounds.maxY - height))
        widget.frame = NSRect(x: x, y: y, width: width, height: height)
        switch kind {
        case .column: activation = NSRect(x: cell.minX, y: table.minY - 3, width: cell.width, height: 4)
        case .rowLeft: activation = NSRect(x: editor.bounds.minX, y: cell.minY, width: table.minX + 1 - editor.bounds.minX, height: cell.height)
        case .rowRight: activation = NSRect(x: table.maxX - 1, y: cell.minY, width: editor.bounds.maxX - table.maxX + 1, height: cell.height)
        case .rowSelection, .columnSelection: activation = .zero
        }
        widget.layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor
        widget.isHidden = false
    }

    /// The table cell whose column owns the strip along the table's top edge.
    func columnSelectionCell(at point: NSPoint) -> ViemTableCellV1? {
        guard let editor, let surface, surface.backend.sourceFormat == .markdown, let snapshot = surface.layoutSnapshot,
              editor.bounds.contains(point), !ownsPointerInteraction(at: point) else { return nil }
        return snapshot.tableCells.first { Self.ownsColumn(point, cell: editor.viewRect($0.rect), table: editor.viewRect($0.table_rect)) }
    }
    /// Begins selecting complete columns, like the row-selection margin does
    /// for rows. Returns false when the point is not in a column strip.
    func beginColumnSelection(at point: NSPoint) -> Bool {
        guard let cell = columnSelectionCell(at: point), let session = surface?.session, let snapshot = surface?.layoutSnapshot,
              let context = try? session.tableContext(at: cell.text_start, documentID: snapshot.info.identity.document_id,
                revision: snapshot.info.identity.document_revision),
              context.flags & UInt32(VIEM_TABLE_IN_TABLE) != 0, context.rows > 0 else { return false }
        endDrag(); dismiss()
        columnGesture = ColumnGesture(table: cell.table_id, anchor: cell.column, rows: context.rows)
        prefersColumnControls = true
        selectColumns(through: cell.column)
        return true
    }
    func dragColumns(to point: NSPoint) {
        guard let editor, let gesture = columnGesture, let snapshot = surface?.layoutSnapshot else { return }
        func distance(_ cell: ViemTableCellV1) -> CGFloat {
            let rect = editor.viewRect(cell.rect)
            return max(rect.minX - point.x, 0, point.x - rect.maxX)
        }
        let nearest = snapshot.tableCells.filter { $0.table_id == gesture.table }.min { a, b in
            let da = distance(a), db = distance(b)
            return da == db ? a.column > b.column : da < db
        }
        if let nearest { selectColumns(through: nearest.column) }
    }
    func endColumnSelection() {
        columnGesture = nil
        invalidate()
    }
    /// Row-margin gestures show row controls even when every cell is selected.
    func beganRowSelection() {
        prefersColumnControls = false
        dismiss()
    }
    private func selectColumns(through column: UInt64) {
        guard let gesture = columnGesture, let surface, let session = surface.session, let snapshot = surface.layoutSnapshot else { return }
        // Like row selection, the caret ends the selection in the drag's
        // direction: at the end of the last column's final cell, or at the
        // start of the first column's header cell.
        let forward = column >= gesture.anchor
        var selection = ViemTableSelectionV1(); selection.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
        selection.active = 1; selection.document_id = snapshot.info.identity.document_id
        selection.document_revision = snapshot.info.identity.document_revision; selection.table_id = gesture.table
        selection.anchor_row = forward ? 0 : gesture.rows - 1; selection.anchor_column = gesture.anchor
        selection.active_row = forward ? gesture.rows - 1 : 0; selection.active_column = column
        if let current = try? session.tableSelection(), current.active != 0, current.table_id == selection.table_id,
           current.document_revision == selection.document_revision, current.anchor_row == selection.anchor_row,
           current.anchor_column == selection.anchor_column, current.active_row == selection.active_row,
           current.active_column == column { return }
        surface.performInput { _ = try session.selectTableCells(selection) }
    }
    @objc private func widgetAction(_ sender: NSButton) { if let target { perform(EVTableMenuAction(target, UInt32(sender.tag))) } }
    @objc private func alignmentPopup(_ sender: NSButton) {
        guard let target else { return }
        let menu = alignmentMenu(target); menu.delegate = self
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.maxY + 2), in: sender)
    }
    func menuWillOpen(_ menu: NSMenu) { popupTracking = true }
    func menuDidClose(_ menu: NSMenu) { popupTracking = false; invalidate() }
    private func alignmentMenu(_ context: ViemTableContextV1) -> NSMenu {
        let menu = NSMenu(title: Self.alignmentName(context))
        for (name, symbol, alignment) in [("Left", "text.alignleft", UInt32(1)), ("Center", "text.aligncenter", 2), ("Right", "text.alignright", 3)] {
            let item = menuItem(name, context: context, action: 7, alignment: alignment)
            item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: name)
            // Columns with different alignments mark no choice.
            item.state = max(1, context.alignment) == alignment ? .on : .off; menu.addItem(item)
        }
        return menu
    }
    func appendMenu(to menu: NSMenu, event: NSEvent) {
        guard let editor, let surface, let session = surface.session, surface.backend.sourceFormat == .markdown else { return }
        var context: ViemTableContextV1?
        let selection = try? session.tableContext()
        if event.type == .keyDown { context = selection }
        else if let snapshot = surface.layoutSnapshot, let cell = snapshot.tableCells.first(where: { editor.viewRect($0.rect).contains(editor.convert(event.locationInWindow, from: nil)) }) {
            context = try? session.tableContext(at: cell.text_start, documentID: snapshot.info.identity.document_id, revision: snapshot.info.identity.document_revision)
            // Clicking inside selected complete rows or columns targets them.
            if let selection, selection.flags & UInt32(VIEM_TABLE_ROW_SELECTION | VIEM_TABLE_COLUMN_SELECTION) != 0,
               selection.table_id == cell.table_id, (selection.row..<selection.row_end).contains(cell.row),
               (selection.column..<selection.column_end).contains(cell.column) { context = selection }
        }
        guard let context, context.flags & 2 != 0 else { return }
        let rowGroup = context.flags & UInt32(VIEM_TABLE_ROW_SELECTION) != 0
        let columnGroup = context.flags & UInt32(VIEM_TABLE_COLUMN_SELECTION) != 0
        let rows = Self.rowsName(context), columns = Self.columnsName(context)
        menu.addItem(.separator())
        if rowGroup || !columnGroup {
            if context.row > 0 { menu.addItem(menuItem("Insert row above \(rows)", context: context, action: 1)) }
            menu.addItem(menuItem("Insert row below \(rows)", context: context, action: 2))
            // The header row alone has no delete; a selected group clears it.
            if rowGroup || context.row > 0 { menu.addItem(menuItem("Delete \(rows)", context: context, action: 3)) }
        }
        if columnGroup || !rowGroup {
            menu.addItem(menuItem("Insert column left of \(columns)", context: context, action: 4))
            menu.addItem(menuItem("Insert column right of \(columns)", context: context, action: 5))
            menu.addItem(menuItem("Delete \(columns)", context: context, action: 6))
            let alignment = NSMenuItem(title: Self.alignmentName(context), action: nil, keyEquivalent: "")
            alignment.submenu = alignmentMenu(context); menu.addItem(alignment)
        }
    }
    private func menuItem(_ title: String, context: ViemTableContextV1, action: UInt32, alignment: UInt32 = 0) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: #selector(menuAction(_:)), keyEquivalent: "")
        item.target = self; item.representedObject = EVTableMenuAction(context, action, alignment); return item
    }
    @objc private func menuAction(_ item: NSMenuItem) { if let action = item.representedObject as? EVTableMenuAction { perform(action) } }
    private func perform(_ action: EVTableMenuAction) {
        guard let surface, let session = surface.session else { return }
        surface.performInput { _ = try session.tableAction(action.action, alignment: action.alignment, expected: action.context) }
        hide(); invalidate(); editor?.window?.makeFirstResponder(editor)
    }

    func beginDrag(at point: NSPoint) {
        cellDrag = false; dragAnchor = nil; dragIdentity = nil; dragOrigin = point
        guard let editor, let surface, surface.backend.sourceFormat == .markdown, let snapshot = surface.layoutSnapshot else { return }
        dragAnchor = snapshot.tableCells.first { editor.viewRect($0.rect).contains(point) }; dragIdentity = snapshot.info.identity
    }
    func endDrag() { dragAnchor = nil; dragIdentity = nil; cellDrag = false }
    func drag(to point: NSPoint) -> Bool {
        guard let editor, let surface, let session = surface.session, let anchor = dragAnchor, let identity = dragIdentity, let snapshot = surface.layoutSnapshot,
              snapshot.info.identity.document_id == identity.document_id && snapshot.info.identity.document_revision == identity.document_revision else { return false }
        guard hypot(point.x - dragOrigin.x, point.y - dragOrigin.y) >= 4 else { return false }
        let cells = snapshot.tableCells.filter { $0.table_id == anchor.table_id }
        let layoutPoint = editor.layoutPoint(fromViewPoint: point)
        let destination = cells.min { a, b in
            func distance(_ c: ViemTableCellV1) -> CGFloat {
                let r = NSRect(x: CGFloat(c.rect.x), y: CGFloat(c.rect.y), width: CGFloat(c.rect.width), height: CGFloat(c.rect.height))
                return hypot(max(r.minX - layoutPoint.x, 0, layoutPoint.x - r.maxX), max(r.minY - layoutPoint.y, 0, layoutPoint.y - r.maxY))
            }
            let da = distance(a), db = distance(b)
            return da == db ? (a.row == b.row ? a.column > b.column : a.row > b.row) : da < db
        }
        guard let destination else { return false }
        if destination.cell_id != anchor.cell_id { cellDrag = true }
        if !cellDrag {
            if anchor.text_start == anchor.text_end { cellDrag = true }
            else if let caret = snapshot.carets.last(where: { $0.text_offset == anchor.text_end }),
                    let row = snapshot.rows.first(where: { $0.row_index == caret.row_index }) {
                let end = editor.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(caret.x), y: CGFloat(row.y)))
                let rect = editor.viewRect(anchor.rect)
                let previous = snapshot.carets.last { $0.row_index == caret.row_index && $0.text_offset < anchor.text_end }
                let rtl = previous.map { $0.x > caret.x } ?? false
                if point.y >= end.y && point.y <= end.y + CGFloat(row.line_advance) && (rtl ? point.x < end.x - 4 : point.x > end.x + 4) && rect.contains(point) { cellDrag = true }
            }
        }
        guard cellDrag else { return false }
        var selection = ViemTableSelectionV1(); selection.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
        selection.active = 1; selection.document_id = identity.document_id; selection.document_revision = identity.document_revision
        selection.table_id = anchor.table_id; selection.anchor_row = anchor.row; selection.anchor_column = anchor.column
        selection.active_row = destination.row; selection.active_column = destination.column
        surface.performInput { _ = try session.selectTableCells(selection) }; return true
    }
    func accessibilityTables() -> [NSAccessibilityElement] {
        guard let editor, let window = editor.window, let surface, let session = surface.session,
              surface.backend.sourceFormat == .markdown, let snapshot = surface.layoutSnapshot else { return [] }
        let selection = try? session.tableSelection()
        var tables: [NSAccessibilityElement] = []
        for (_, cells) in Dictionary(grouping: snapshot.tableCells, by: \.table_id).sorted(by: { $0.key < $1.key }) {
            guard let first = cells.first,
                  let context = try? session.tableContext(at: first.text_start, documentID: snapshot.info.identity.document_id, revision: snapshot.info.identity.document_revision) else { continue }
            let table = NSAccessibilityElement()
            table.setAccessibilityElement(true); table.setAccessibilityRole(.table); table.setAccessibilityParent(editor)
            table.setAccessibilityLabel("Table, \(context.columns) columns, \(context.rows) rows including header")
            table.setAccessibilityRowCount(Int(context.rows)); table.setAccessibilityColumnCount(Int(context.columns))
            table.setAccessibilityFrame(window.convertToScreen(editor.convert(editor.viewRect(first.table_rect), to: nil)))
            var rows: [NSAccessibilityElement] = [], headers: [NSAccessibilityElement] = [], selected: [NSAccessibilityElement] = []
            for (rowIndex, rowCells) in Dictionary(grouping: cells, by: \.row).sorted(by: { $0.key < $1.key }) {
                let row = NSAccessibilityElement(); row.setAccessibilityElement(true); row.setAccessibilityRole(.row); row.setAccessibilityParent(table)
                row.setAccessibilityLabel(rowIndex == 0 ? "Header row" : "Row \(rowIndex + 1)")
                var children: [NSAccessibilityElement] = []
                for cell in rowCells.sorted(by: { $0.column < $1.column }) {
                    let element = EVAccessibleTableCell(); element.setAccessibilityElement(true); element.setAccessibilityRole(.cell); element.setAccessibilityParent(row)
                    element.setAccessibilityLabel("\(cell.row == 0 ? "Header" : "Row \(cell.row + 1)"), column \(cell.column + 1)")
                    element.readValue = { [weak surface] in
                        guard surface?.documentState.document_revision == snapshot.info.identity.document_revision else { return nil }
                        return surface?.formattedText(in: Int(cell.text_start)..<Int(cell.text_end))
                    }
                    element.focusCell = { [weak surface, weak editor] in
                        guard let surface, let session = surface.session, surface.documentState.document_revision == snapshot.info.identity.document_revision else { return false }
                        var point = ViemLayoutCaretPointV1(); point.document_revision = snapshot.info.identity.document_revision
                        point.text_offset = cell.text_start; point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
                        surface.performInput { _ = try session.placeCursor(point, extendSelection: false) }
                        editor?.window?.makeFirstResponder(editor); return true
                    }
                    element.setAccessibilityRowIndexRange(NSRange(location: Int(cell.row), length: 1))
                    element.setAccessibilityColumnIndexRange(NSRange(location: Int(cell.column), length: 1))
                    element.setAccessibilityFrame(window.convertToScreen(editor.convert(editor.viewRect(cell.rect), to: nil)))
                    if let selection, selection.active != 0, selection.table_id == cell.table_id,
                       cell.row >= min(selection.anchor_row, selection.active_row), cell.row <= max(selection.anchor_row, selection.active_row),
                       cell.column >= min(selection.anchor_column, selection.active_column), cell.column <= max(selection.anchor_column, selection.active_column) {
                        element.setAccessibilitySelected(true); selected.append(element)
                    }
                    var actions: [(String, UInt32, UInt32)] = [("Insert row below", 2, 0), ("Insert column left", 4, 0), ("Insert column right", 5, 0), ("Delete column", 6, 0), ("Align column left", 7, 1), ("Align column center", 7, 2), ("Align column right", 7, 3), ("Use default column alignment", 7, 0)]
                    if cell.row > 0 { actions.insert(contentsOf: [("Insert row above", 1, 0), ("Delete row", 3, 0)], at: 0) }
                    element.setAccessibilityCustomActions(actions.map { name, action, alignment in
                        NSAccessibilityCustomAction(name: name) { [weak self] in
                            MainActor.assumeIsolated {
                                guard let self, let session = self.surface?.session,
                                      let current = try? session.tableContext(at: cell.text_start,
                                        documentID: snapshot.info.identity.document_id, revision: snapshot.info.identity.document_revision) else { return false }
                                self.perform(EVTableMenuAction(current, action, alignment)); return true
                            }
                        }
                    })
                    if cell.row == 0 { headers.append(element) }
                    children.append(element)
                }
                if let firstCell = rowCells.first {
                    let rowRect = rowCells.dropFirst().reduce(editor.viewRect(firstCell.rect)) { $0.union(editor.viewRect($1.rect)) }
                    row.setAccessibilityFrame(window.convertToScreen(editor.convert(rowRect, to: nil)))
                }
                row.setAccessibilityChildren(children); rows.append(row)
            }
            table.setAccessibilityRows(rows); table.setAccessibilityChildren(rows)
            table.setAccessibilityColumnHeaderUIElements(headers); table.setAccessibilitySelectedCells(selected)
            tables.append(table)
        }
        return tables
    }

    func selectionRects(_ snapshot: EVLayoutExport) -> [NSRect]? {
        guard let editor, let selection = try? surface?.session?.tableSelection(), selection.active != 0,
              selection.document_revision == snapshot.info.identity.document_revision else { return nil }
        return snapshot.tableCells.filter { cell in
            cell.table_id == selection.table_id && cell.row >= min(selection.anchor_row, selection.active_row)
                && cell.row <= max(selection.anchor_row, selection.active_row)
                && cell.column >= min(selection.anchor_column, selection.active_column)
                && cell.column <= max(selection.anchor_column, selection.active_column)
        }.map { editor.viewRect($0.rect) }
    }

    @discardableResult
    func drawSelection(_ snapshot: EVLayoutExport, in context: CGContext) -> Bool {
        guard let rectangles = selectionRects(snapshot) else { return false }
        context.setFillColor(NSColor.selectedTextBackgroundColor.withAlphaComponent(0.38).cgColor)
        for rect in rectangles { context.fill(rect) }
        return true
    }
}
