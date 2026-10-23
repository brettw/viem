import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVTableEditingTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var content: EVClipboardRepresentations?
        var viemGeneration: UInt64 = 1
        var viemIsWritable: Bool { true }
        func viemString() -> String? { content?.plainText }
        func viemCanReadString() -> Bool { content != nil }
        func viemClearContents() -> Int { content = nil; viemGeneration += 1; return Int(viemGeneration) }
        func viemSetString(_ string: String) -> Bool { content = EVClipboardRepresentations(plainText: string); viemGeneration += 1; return true }
        func viemData(forType type: NSPasteboard.PasteboardType) -> Data? {
            type == EVClipboardRepresentations.fragmentType ? content?.fragment : type == .rtf ? content?.richText : nil
        }
        func viemWrite(_ content: EVClipboardRepresentations) -> Bool { self.content = content; viemGeneration += 1; return true }
    }

    private func surface(_ text: String, code: Bool = false) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-tables-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        try backend.read(source: Data(text.utf8), typeName: code ? EVDocument.codeType : EVDocument.markdownType,
            filename: "notes.md", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded(); surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 400); surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    func testInsertTableCountsBodyRowsAndRestoresExactBytesWithUndo() throws {
        let (backend, surface, session) = try surface("Before")
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))
        let context = try session.tableContext()
        XCTAssertNotEqual(context.flags & UInt32(VIEM_TABLE_CAN_INSERT), 0)
        _ = try session.insertTable(columns: 3, bodyRows: 4, expected: context.selection)
        surface.refreshPresentation()
        let after = try session.tableContext()
        XCTAssertEqual(after.columns, 3); XCTAssertEqual(after.rows, 5)
        XCTAssertEqual(after.row, 0); XCTAssertEqual(after.column, 0)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("| --- | --- | --- |"))
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("Before".utf8))
        _ = try session.redo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
    }

    func testNativeStructuralActionsRetainAlignmentAndUndoExactSpelling() throws {
        let source = "| A | B |\n| :-- | --: |\n| one | two |"
        let (backend, surface, session) = try surface(source)
        let original = try session.tableContext()
        XCTAssertEqual(original.rows, 2); XCTAssertEqual(original.columns, 2)
        XCTAssertNoThrow(try session.tableAction(UInt32(VIEM_TABLE_INSERT_ROW_BELOW), expected: original))
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableContext().rows, 3)
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        let context = try session.tableContext()
        XCTAssertNoThrow(try session.tableAction(UInt32(VIEM_TABLE_SET_ALIGNMENT), alignment: UInt32(VIEM_TABLE_ALIGN_CENTER), expected: context))
        XCTAssertEqual(try session.tableContext().alignment, UInt32(VIEM_TABLE_ALIGN_CENTER))
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testHoverDismissesWhenSelectionChangesAndContextActionsNameTheirTarget() throws {
        let (_, surface, _) = try surface("| Header | Other |\n| --- | --- |\n| body | value |")
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let header = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 0 })
        let rect = editor.viewRect(header.rect), table = editor.viewRect(header.table_rect)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        interaction.moved(to: NSPoint(x: rect.midX, y: table.minY))
        XCTAssertFalse(widget.isHidden)
        XCTAssertLessThanOrEqual(widget.frame.minY, rect.maxY + 4, "A flipped widget stays near its cell, even for a long table")
        XCTAssertTrue(widget.arrangedSubviews.contains { $0.accessibilityLabel() == "Column 1 alignment" })
        editor.setAccessibilitySelectedTextRange(NSRange(location: Int(header.text_start), length: 1))
        interaction.invalidate()
        XCTAssertTrue(widget.isHidden, "Caret/selection changes retire the widget's exact expected selection")

        let key = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 0))
        let menu = NSMenu()
        interaction.appendMenu(to: menu, event: key)
        XCTAssertFalse(menu.items.contains { $0.title.hasPrefix("Insert row above") })
        XCTAssertFalse(menu.items.contains { $0.title.hasPrefix("Delete") && $0.title.hasSuffix("row") },
            "The header row by itself has no delete")
        XCTAssertTrue(menu.items.contains { $0.title == "Insert row below header row" })
        XCTAssertTrue(menu.items.contains { $0.title == "Insert column left of column 1" })
        let align = try XCTUnwrap(menu.items.first { $0.title == "Column 1 alignment" }?.submenu)
        XCTAssertEqual(align.items.map(\.title), ["Left", "Center", "Right"])
    }

    func testRowHoverWorksOnBothSidesWithoutChangingSelectionAndKeepsItsTargetIntoWidget() throws {
        let text = "| Header | Other |\n| --- | --- |\n| first | one |\n| second | two |\n| third | three |"
        let (backend, surface, session) = try surface(text)
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let rows = snapshot.tableCells.filter { $0.column == 0 }.sorted { $0.row < $1.row }
        XCTAssertEqual(rows.count, 4)
        let selection = try session.listSelection()
        let presentation = surface.viewPresentation
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        for right in [false, true] {
            interaction.hide()
            for cell in rows {
                let table = editor.viewRect(cell.table_rect), rect = editor.viewRect(cell.rect)
                let point = NSPoint(x: right ? table.maxX : table.minX, y: rect.midY)
                interaction.moved(to: point)
                XCTAssertFalse(widget.isHidden)
                let labels = widget.arrangedSubviews.compactMap { $0.accessibilityLabel() }
                XCTAssertEqual(labels, cell.row == 0
                    ? ["Insert row below row 1"]
                    : ["Insert row above row \(cell.row + 1)", "Delete row \(cell.row + 1)", "Insert row below row \(cell.row + 1)"])
                if right { XCTAssertGreaterThanOrEqual(widget.frame.minX, table.maxX, "Prefer the side that activated the widget when there is room") }
                // Cross the gap and enter a button without rebuilding or retargeting it.
                let button = try XCTUnwrap(widget.arrangedSubviews.last as? NSButton)
                let entryX = point.x < widget.frame.midX ? widget.frame.minX - 1 : widget.frame.maxX + 1
                interaction.moved(to: NSPoint(x: entryX, y: widget.frame.midY))
                interaction.moved(to: NSPoint(x: widget.frame.midX, y: widget.frame.midY))
                XCTAssertFalse(widget.isHidden)
                XCTAssertTrue(widget.arrangedSubviews.last === button)
                XCTAssertTrue(try session.listSelection().isSameSelection(as: selection))
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, presentation.cursor_utf8_offset)
                XCTAssertEqual(surface.viewPresentation.mode, presentation.mode)
            }
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(text.utf8))

        interaction.hide()
        let target = try XCTUnwrap(rows.first { $0.row == 1 })
        let rect = editor.viewRect(target.rect), table = editor.viewRect(target.table_rect)
        interaction.moved(to: NSPoint(x: table.maxX + 20, y: rect.midY))
        let insert = try XCTUnwrap(widget.arrangedSubviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "Insert row below row 2" })
        insert.performClick(nil)
        XCTAssertEqual(try session.tableContext().rows, 5)
        XCTAssertEqual(try session.tableContext().row, 2, "An explicit right-side action affects the hovered row, even when the caret was elsewhere")
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(text.utf8))
    }

    func testRowHoverEdgesCornersAndClippingUseActualTableBoundaries() throws {
        let (_, surface, _) = try surface("| A | B | C |\n| --- | --- | --- |\n| one | two | three |\n| four | five | six |")
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let first = try XCTUnwrap(snapshot.tableCells.first)
        let table = editor.viewRect(first.table_rect)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        func labels(at point: NSPoint) -> [String] {
            interaction.hide(); interaction.moved(to: point)
            return widget.isHidden ? [] : widget.arrangedSubviews.compactMap { $0.accessibilityLabel() }
        }
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: table.minY)).contains("Column 3 alignment"))
        XCTAssertTrue(labels(at: NSPoint(x: table.minX, y: table.minY)).contains("Column 1 alignment"))
        let secondColumn = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 1 })
        XCTAssertTrue(labels(at: NSPoint(x: editor.viewRect(secondColumn.rect).minX, y: table.minY)).contains("Column 2 alignment"))
        let secondRow = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 })
        let secondRect = editor.viewRect(secondRow.rect)
        for x in [table.minX, table.maxX] {
            XCTAssertTrue(labels(at: NSPoint(x: x, y: secondRect.minY)).contains("Delete row 2"))
            XCTAssertTrue(labels(at: NSPoint(x: x, y: table.maxY)).contains("Delete row 3"))
        }
        // Both row zones reach the editor's edges.
        for x in [table.maxX + 21, editor.bounds.maxX - 1, editor.bounds.minX, table.minX] {
            XCTAssertTrue(labels(at: NSPoint(x: x, y: secondRect.midY)).contains("Delete row 2"), "x=\(x)")
            XCTAssertEqual(editor.isRowSelectionPoint(NSPoint(x: x, y: secondRect.midY)), x <= table.minX, "x=\(x)")
        }
        XCTAssertTrue(labels(at: NSPoint(x: table.minX + 2, y: secondRect.midY)).isEmpty)
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX - 2, y: secondRect.midY)).isEmpty)

        let bounds = editor.bounds
        defer { editor.bounds = bounds }
        editor.bounds.size.width = table.maxX + 25
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: secondRect.midY)).contains("Delete row 2"))
        XCTAssertGreaterThanOrEqual(widget.frame.minX, editor.bounds.minX)
        XCTAssertLessThanOrEqual(widget.frame.maxX, editor.bounds.maxX)
        editor.bounds.size.width = table.maxX - 10
        XCTAssertTrue(labels(at: NSPoint(x: editor.bounds.maxX - 1, y: secondRect.midY)).isEmpty,
            "A clipped right edge does not become a hotspot at the viewport boundary")
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: secondRect.midY)).isEmpty,
            "Offscreen table geometry cannot activate a widget")
    }

    private func buttons(_ widget: NSStackView) -> [String] { widget.arrangedSubviews.compactMap { $0.accessibilityLabel() } }
    private func press(_ label: String, in widget: NSStackView) throws {
        try XCTUnwrap(widget.arrangedSubviews.compactMap { $0 as? NSButton }.first { $0.accessibilityLabel() == label }).performClick(nil)
    }

    func testSelectedRowsShowGroupControlsAtTheActiveRowAndInsertionsKeepTheSelection() throws {
        let text = "| A | B |\n| --- | --- |\n| one | 1 |\n| two | 2 |\n| three | 3 |"
        let (backend, surface, session) = try surface(text)
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        let first = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 })
        let second = try XCTUnwrap(snapshot.tableCells.first { $0.row == 2 })
        let margin = NSPoint(x: editor.textViewportRect.minX + 1, y: editor.viewRect(first.rect).midY)
        XCTAssertTrue(editor.isRowSelectionPoint(margin))
        XCTAssertTrue(editor.pointerCursor(at: margin) === EVLineNumberGutter.selectionCursor)
        _ = try session.selectPointerRow(y: CGFloat(first.rect.y + first.rect.height / 2), in: snapshot.info, extending: false)
        _ = try session.selectPointerRow(y: CGFloat(second.rect.y + second.rect.height / 2), in: snapshot.info, extending: true)
        surface.refreshPresentation(); interaction.invalidate()
        XCTAssertFalse(widget.isHidden)
        XCTAssertEqual(buttons(widget), ["Insert row above rows 2–3", "Delete rows 2–3", "Insert row below rows 2–3"])
        XCTAssertEqual(widget.frame.midY, editor.viewRect(second.rect).midY, accuracy: 1, "Controls follow the active row")
        interaction.moved(to: NSPoint(x: margin.x, y: editor.viewRect(snapshot.tableCells.first { $0.row == 3 }!.rect).midY))
        XCTAssertEqual(buttons(widget), ["Insert row above rows 2–3", "Delete rows 2–3", "Insert row below rows 2–3"],
            "Hovering another row keeps the selection's controls")

        try press("Insert row above rows 2–3", in: widget)
        surface.refreshPresentation()
        let selection = try session.tableSelection()
        XCTAssertEqual([selection.anchor_row, selection.active_row].sorted(), [2, 3])
        XCTAssertEqual(try session.tableContext().rows, 5)
        XCTAssertEqual(buttons(widget), ["Insert row above rows 3–4", "Delete rows 3–4", "Insert row below rows 3–4"])
        try press("Delete rows 3–4", in: widget)
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableSelection().active, 0)
        XCTAssertTrue(widget.isHidden)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("| A | B |\n| --- | --- |\n|  |  |\n| three | 3 |".utf8))
    }

    func testSelectedRowsIncludingTheHeaderClearItWithoutInsertingAbove() throws {
        let (backend, surface, session) = try surface("| A | B |\n| --- | --- |\n| one | 1 |\n| two | 2 |")
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        for (row, y) in [(0, false), (1, true)] {
            let cell = try XCTUnwrap(snapshot.tableCells.first { $0.row == row })
            _ = try session.selectPointerRow(y: CGFloat(cell.rect.y + cell.rect.height / 2), in: snapshot.info, extending: y)
        }
        surface.refreshPresentation(); interaction.invalidate()
        XCTAssertEqual(buttons(widget), ["Delete rows 1–2", "Insert row below rows 1–2"])
        try press("Delete rows 1–2", in: widget)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("|  |  |\n| --- | --- |\n| two | 2 |".utf8))
    }

    func testColumnStripSelectsColumnsAndShowsGroupControlsAtTheLastColumn() throws {
        let text = "Intro\n\nRoom above\n\n| A | B | C |\n| :-- | :-: | --: |\n| one | two | three |\n| four | five | six |"
        let (backend, surface, session) = try surface(text)
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        func strip(_ column: UInt64) throws -> NSPoint {
            let cell = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == column })
            return NSPoint(x: editor.viewRect(cell.rect).midX, y: editor.viewRect(cell.table_rect).minY - 1)
        }
        XCTAssertTrue(editor.pointerCursor(at: try strip(1)) === EVTableInteraction.columnSelectionCursor)
        XCTAssertFalse(editor.pointerCursor(at: NSPoint(x: try strip(1).x, y: try strip(1).y + 8)) === EVTableInteraction.columnSelectionCursor)
        XCTAssertEqual(EVTableInteraction.columnSelectionCursor.hotSpot, NSPoint(x: 7.5, y: 19))

        XCTAssertTrue(interaction.beginColumnSelection(at: try strip(1)))
        interaction.dragColumns(to: try strip(2))
        XCTAssertTrue(widget.isHidden, "Controls wait for the gesture to end")
        interaction.endColumnSelection()
        let selection = try session.tableSelection()
        XCTAssertEqual([selection.anchor_column, selection.active_column], [1, 2])
        XCTAssertEqual([selection.anchor_row, selection.active_row], [0, 2])
        let bottom = try XCTUnwrap(snapshot.tableCells.first { $0.row == 2 && $0.column == 2 })
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, bottom.text_end,
            "Like row selection, a rightward drag puts the caret at the end of the last selected cell")
        XCTAssertEqual(buttons(widget), ["Insert column left of columns 2–3", "Columns 2–3 alignment", "Delete columns 2–3",
            "Insert column right of columns 2–3"])
        let last = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 2 })
        XCTAssertEqual(widget.frame.midX, editor.viewRect(last.rect).midX, accuracy: 1)
        XCTAssertLessThanOrEqual(widget.frame.maxY, editor.viewRect(last.table_rect).minY)

        // Alignment applies to every selected column and keeps the selection.
        let key = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 0))
        let menu = NSMenu()
        interaction.appendMenu(to: menu, event: key)
        XCTAssertFalse(menu.items.contains { $0.title.hasPrefix("Delete row") || $0.title.hasPrefix("Insert row") })
        let align = try XCTUnwrap(menu.items.first { $0.title == "Columns 2–3 alignment" }?.submenu)
        XCTAssertEqual(align.items.map(\.state), [.off, .off, .off], "Mixed alignments mark no choice")
        align.performActionForItem(at: try XCTUnwrap(align.items.firstIndex { $0.title == "Center" }))
        surface.refreshPresentation()
        XCTAssertTrue(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self)
            .contains("| :-- | :-: | :--: |"))
        XCTAssertEqual(try session.tableContext().alignment, UInt32(VIEM_TABLE_ALIGN_CENTER))
        XCTAssertEqual([try session.tableSelection().anchor_column, try session.tableSelection().active_column], [1, 2])
        interaction.invalidate()
        XCTAssertEqual(buttons(widget)[1], "Columns 2–3 alignment")

        try press("Insert column right of columns 2–3", in: widget)
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableContext().columns, 4)
        XCTAssertEqual([try session.tableSelection().anchor_column, try session.tableSelection().active_column], [1, 2])
        try press("Delete columns 2–3", in: widget)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("Intro\n\nRoom above\n\n| A |   |\n| :-- | :---: |\n| one |   |\n| four |   |".utf8))

        // A single selected column keeps its alignment control.
        _ = try session.undo(); surface.refreshPresentation()
        let current = try XCTUnwrap(surface.layoutSnapshot)
        let header = try XCTUnwrap(current.tableCells.first { $0.row == 0 && $0.column == 0 })
        XCTAssertTrue(interaction.beginColumnSelection(at: NSPoint(x: editor.viewRect(header.rect).midX, y: editor.viewRect(header.table_rect).minY)))
        interaction.endColumnSelection()
        XCTAssertEqual(buttons(widget), ["Insert column left of column 1", "Column 1 alignment", "Delete column 1", "Insert column right of column 1"])

        // A leftward drag puts the caret at the start of the first column's header.
        let third = try XCTUnwrap(current.tableCells.first { $0.row == 0 && $0.column == 2 })
        XCTAssertTrue(interaction.beginColumnSelection(at: NSPoint(x: editor.viewRect(third.rect).midX, y: editor.viewRect(third.table_rect).minY)))
        let second = try XCTUnwrap(current.tableCells.first { $0.row == 0 && $0.column == 1 })
        interaction.dragColumns(to: NSPoint(x: editor.viewRect(second.rect).midX, y: editor.viewRect(second.table_rect).minY))
        interaction.endColumnSelection()
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, second.text_start)
        XCTAssertEqual(buttons(widget).first, "Insert column left of columns 2–3")
    }

    func testRowHoverDoesNotShowWidgetsWithPreviewOff() throws {
        let (_, surface, _) = try surface("| A | B |\n| --- | --- |\n| one | two |")
        let editor = surface.editorView
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        let cells = try XCTUnwrap(surface.layoutSnapshot).tableCells
        XCTAssertFalse(cells.isEmpty)
        surface.formattingToolbar.formattedView.performClick(nil)
        for cell in cells {
            let table = editor.viewRect(cell.table_rect), rect = editor.viewRect(cell.rect)
            for x in [table.minX, table.maxX, table.maxX + 10] {
                interaction.moved(to: NSPoint(x: x, y: rect.midY))
                XCTAssertTrue(widget.isHidden)
            }
        }
    }

    func testTableGeometryAndCellSelectionRemainPortableAndDisableBlockControls() throws {
        let (_, surface, session) = try surface("| A | B |\n| --- | --- |\n| one | two |")
        let snapshot = try session.layoutExport()
        XCTAssertEqual(snapshot.tableCells.count, 4)
        let first = try XCTUnwrap(snapshot.tableCells.first)
        let second = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 1 })
        XCTAssertEqual(first.rect.y, second.rect.y, accuracy: 0.01)
        XCTAssertGreaterThan(second.rect.x, first.rect.x)
        var selection = ViemTableSelectionV1(); selection.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
        selection.active = 1; selection.document_id = snapshot.info.identity.document_id; selection.document_revision = snapshot.info.identity.document_revision
        selection.table_id = first.table_id; selection.anchor_row = 0; selection.anchor_column = 0; selection.active_row = 1; selection.active_column = 1
        _ = try session.selectTableCells(selection)
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableSelection().active, 1)
        XCTAssertFalse(surface.formattingToolbar.paragraphStyle.isEnabled)
        XCTAssertFalse(surface.formattingToolbar.commandButtons[.bulletedList]?.isEnabled ?? true)
        XCTAssertTrue(surface.formattingToolbar.codeBlock.isHidden)
        selection.anchor_column = 1
        _ = try session.selectTableCells(selection)
        surface.refreshPresentation()
        XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "B\ntwo")
        XCTAssertEqual(surface.editorView.accessibilitySelectedTextRanges()?.count, 2)
        let current = try XCTUnwrap(surface.layoutSnapshot)
        let rectangles = surface.editorView.selectionRectsForDrawing(in: current)
        XCTAssertEqual(rectangles.count, 2)
        for (rect, cell) in zip(rectangles, current.tableCells.filter { $0.column == 1 }) {
            XCTAssertEqual(rect, surface.editorView.viewRect(cell.rect), "Selection damage includes padding and empty cell area")
        }

    }
    func testIdleTableWidthsRefineOnWorkerAndStopWithoutChangingSourceOrCaret() async throws {
        let source = "| A | B |\n| --- | --- |\n" + (0..<2000).map { "| row \($0) | \($0 == 1999 ? String(repeating: "wide ", count: 30) : "small") |" }.joined(separator: "\n")
        let (backend, surface, session) = try surface(source)
        let before = surface.viewPresentation
        for _ in 0..<100 {
            if session.tableWidthRefinement.started > 0 { break }
            await Task.yield()
        }
        // A small viewport change must cancel its captured visible request,
        // then resume discovery rather than treating stale work as a failure.
        _ = try session.setViewportOrigin(left: 0, top: 6)
        surface.refreshPresentation()
        let viewport = surface.viewportState
        for _ in 0..<750 {
            if session.tableWidthRefinement.isIdle && session.tableWidthRefinement.started > 0 { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertTrue(session.tableWidthRefinement.isIdle)
        XCTAssertNil(session.tableWidthRefinement.lastError)
        XCTAssertGreaterThan(session.tableWidthRefinement.installed, 0, "started=\(session.tableWidthRefinement.started), discarded=\(session.tableWidthRefinement.discarded)")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, before.cursor_utf8_offset)
        XCTAssertEqual(surface.viewportState.top, viewport.top, accuracy: 0.01)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        let started = session.tableWidthRefinement.started
        surface.refreshPresentation()
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(session.tableWidthRefinement.started, started, "Idle does not poll or repeat completed discovery")
        session.detach()
    }

    func testNativeCutPasteRoundTripsStyledRectangleWithBreaksAndEmptyCells() throws {
        let text = "# Table interaction test\n\n| Feature | Example | Status |\n| :--- | :---: | ---: |\n| Bold | **bold**<br>Hello, world! | ready |\n| Italic | *italic* | test |\n| Empty | | |\n\nAfter the table.\n"
        for insertMode in [false, true] {
            let (backend, surface, session) = try surface(text)
            let clipboard = Pasteboard(); surface.pasteboard = clipboard
            if insertMode { _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 105) }
            let before = try backend.formattedText()
            let snapshot = try session.layoutExport()
            let cell = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 && $0.column == 1 })
            var selected = ViemTableSelectionV1(); selected.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
            selected.active = 1; selected.document_id = snapshot.info.identity.document_id; selected.document_revision = snapshot.info.identity.document_revision
            selected.table_id = cell.table_id; selected.anchor_row = 1; selected.anchor_column = 1; selected.active_row = 3; selected.active_column = 2
            _ = try session.selectTableCells(selected); surface.refreshPresentation()
            surface.editorView.cutDocumentSelection(nil)
            XCTAssertEqual(surface.statusBarState.message, "", "cut, insert=\(insertMode)")
            XCTAssertNotNil(clipboard.content?.fragment)
            let payload = try XCTUnwrap(clipboard.content?.fragment)
            let json = try XCTUnwrap(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            XCTAssertEqual(json["plain_text"] as? String, clipboard.content?.plainText)
            let matrix = try XCTUnwrap(json["table_cells"] as? [[[String: Any]]])
            XCTAssertEqual(matrix.count, 3); XCTAssertTrue(matrix.allSatisfy { $0.count == 2 })
            XCTAssertEqual(matrix[0][0]["plain_text"] as? String, "bold\nHello, world!")
            XCTAssertEqual(try session.tableContext().row, 1)
            XCTAssertEqual(try session.tableContext().column, 1)

            XCTAssertFalse(try backend.formattedText().contains("Hello, world!"))
            surface.editorView.pasteIntoDocument(nil)
            XCTAssertEqual(surface.statusBarState.message, "", "paste, insert=\(insertMode)")
            XCTAssertEqual(try backend.formattedText(), before, "insert=\(insertMode)")
            let pastedSource = try backend.serializedSource(typeName: EVDocument.markdownType)
            surface.refreshPresentation()
            let current = try XCTUnwrap(surface.layoutSnapshot)
            let pastedCell = try XCTUnwrap(current.tableCells.first { $0.row == 1 && $0.column == 1 })
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: Int(pastedCell.text_start),
                length: Int(pastedCell.text_end - pastedCell.text_start)))
            surface.refreshPresentation()
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "bold\nHello, world!",
                "AX selection must target the pasted multiline cell; requested=\(pastedCell.text_start)..<\(pastedCell.text_end), actual=\(surface.selectedUTF8Ranges()), rows=\(current.rows.map { ($0.text_start, $0.text_end, $0.flags) })")
            for (command, style) in [(EVMenuCommand.bold, UInt32(VIEM_SEMANTIC_STYLE_STRONG)),
                                     (.italic, UInt32(VIEM_SEMANTIC_STYLE_EMPHASIS))] {
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: Int(pastedCell.text_start),
                    length: Int(pastedCell.text_end - pastedCell.text_start)))
                surface.refreshPresentation()
                let capability = try session.semanticStylePresentation(style)
                XCTAssertNotEqual(capability.flags & UInt32(VIEM_SEMANTIC_STYLE_CAN_SET), 0,
                    "Multiline cell style \(style) must remain available after paste: flags=\(capability.flags), kind=\(capability.selection.kind), range=\(capability.selection.text_start)..<\(capability.selection.text_end)\n\(String(decoding: pastedSource, as: UTF8.self))")
                XCTAssertTrue(surface.presentation(for: command).isEnabled)
                _ = try session.setSemanticStyle(style, enabled: true, expected: capability.selection)
                surface.refreshPresentation()
                let applied = try session.semanticStylePresentation(style)
                let styledSource = try backend.serializedSource(typeName: EVDocument.markdownType)
                XCTAssertEqual(applied.state, UInt32(VIEM_SEMANTIC_STYLE_STATE_ON),
                    "style=\(style), before=\(capability.selection.text_start)..<\(capability.selection.text_end), after=\(applied.selection.text_start)..<\(applied.selection.text_end), selected=\(surface.selectedUTF8Ranges())\n\(String(decoding: styledSource, as: UTF8.self))")
                _ = try session.undo(); surface.refreshPresentation()
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), pastedSource)
            }
        }
    }

}
